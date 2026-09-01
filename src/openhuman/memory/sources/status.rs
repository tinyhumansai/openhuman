//! Per-source sync status — chunks ingested, freshness, in-flight progress.
//!
//! # Two halves, and which side each one belongs on (#5560)
//!
//! This used to be `tinymemory_core::sources::status`, one function that read
//! the host's source registry and then ran `SELECT … FROM mem_tree_chunks`
//! against the engine's SQLite. That is two questions with two owners:
//!
//! - **Which key does this configured source stamp on its chunks?**
//!   [`source_id_prefix`] — pure string formatting over the registry entry's
//!   kind, toolkit and connection id. The registry is *this host's* file, so
//!   only this host can answer it; a driver asked to derive the prefix would
//!   need the registry, which is precisely the coupling the contract removes.
//! - **How many rows sit under that key, and how many are still in flight?**
//!   `MemoryChunks::source_ingest_status`, because `chunks_pending` spans the
//!   embedding sidecar and the re-embed skip ledger as well as the chunk row's
//!   own lifecycle column, and nothing outside the driver can see those.
//!
//! [`FreshnessLabel`] is the third piece and it is the caller's, deliberately:
//! it is arithmetic over `last_chunk_at_ms` and a clock, so answering it
//! driver-side would freeze the driver's clock into the reply and a panel
//! rendering the label a minute later would show how fresh the source was when
//! the driver looked.
//!
//! **`MemoryChunks::source_totals` is not a substitute**, and it is worth
//! restating because it looks like one: `SourceTotal` carries `chunk_count` and
//! `most_recent_ms` but no pending count, and it returns the groups that
//! *exist*, so a configured source that has never synced vanishes from the
//! answer rather than appearing idle.

use serde::{Deserialize, Serialize};

use crate::openhuman::config::Config;
use crate::openhuman::memory::api::provider::chunks::SourceIngestQuery;
use crate::openhuman::memory::binding::MemoryBinding;
use crate::openhuman::memory::sources::types::{MemorySourceEntry, SourceKind};

/// How fresh a source's sync is, judged from its newest chunk.
///
/// A verbatim port of the engine's enum, thresholds and all. The serde
/// spelling is `snake_case` and must stay that way: it is what the memory
/// sources panel matches on, and this is a wire type in a response that is
/// otherwise unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessLabel {
    /// Newest chunk is under 30 seconds old.
    Active,
    /// Newest chunk is under 5 minutes old.
    Recent,
    /// Anything older, or nothing synced yet.
    Idle,
}

impl FreshnessLabel {
    /// Label for a source whose newest chunk landed at `last_chunk_at_ms`,
    /// judged at `now_ms`.
    ///
    /// `saturating_sub` rather than `-`: a chunk stamped in the future (a
    /// skewed remote clock on an imported item) yields a non-positive age
    /// rather than an overflow, and reads as [`Self::Active`].
    pub fn from_age_ms(last_chunk_at_ms: Option<i64>, now_ms: i64) -> Self {
        match last_chunk_at_ms {
            None => Self::Idle,
            Some(timestamp) => match now_ms.saturating_sub(timestamp) {
                age if age <= 30_000 => Self::Active,
                age if age <= 5 * 60_000 => Self::Recent,
                _ => Self::Idle,
            },
        }
    }
}

/// One configured source's ingest progress, as the `memory_sources.status_list`
/// RPC has always returned it.
///
/// Field for field and name for name the engine's `SourceStatus`, so the
/// response JSON is unchanged by the move.
#[derive(Clone, Debug, Serialize)]
pub struct SourceStatus {
    pub source_id: String,
    pub chunks_synced: u64,
    pub chunks_pending: u64,
    pub last_chunk_at_ms: Option<i64>,
    pub freshness: FreshnessLabel,
}

impl SourceStatus {
    /// The zero row: a source the store could not be asked about.
    ///
    /// The engine's loop pushed exactly this for a per-source query failure,
    /// and it is what a degraded read answers here too — see [`status_list`].
    fn idle(source_id: String) -> Self {
        Self {
            source_id,
            chunks_synced: 0,
            chunks_pending: 0,
            last_chunk_at_ms: None,
            freshness: FreshnessLabel::Idle,
        }
    }
}

/// The literal prefix of the chunk-ingest key a source's rows carry.
///
/// The scheme is set by the ingest paths, not chosen here: reader-backed kinds
/// key chunks `mem_src:{source.id}:{item}`, and the Composio sync keys them
/// `{toolkit}:{connection_id}:{document_id}`.
///
/// Matching a Composio source on its toolkit alone would sweep in every *other*
/// connection of that toolkit — two Gmail accounts would each report the
/// other's chunks as their own — so the connection narrows it. A Composio entry
/// without a connection id does not pass validation; the toolkit-only fallback
/// is there so a malformed row degrades to a wide match rather than to no match
/// at all.
///
/// # No `%`, unlike the engine's twin
///
/// The engine's `source_id_prefix` returned a `LIKE` pattern (`mem_src:{id}:%`)
/// because its caller passed it straight to SQL. [`SourceIngestQuery`] takes a
/// **literal** prefix — the driver places its own wildcard and escapes any
/// metacharacter in what it was given — so the trailing `%` is dropped and the
/// trailing separator, which is what stops `mem_src:src_a:` also counting
/// `mem_src:src_ab:`, is kept.
pub(crate) fn source_id_prefix(source: &MemorySourceEntry) -> String {
    match source.kind {
        SourceKind::Composio => {
            match (source.toolkit.as_deref(), source.connection_id.as_deref()) {
                (Some(toolkit), Some(connection_id)) => format!("{toolkit}:{connection_id}:"),
                // A connection-less entry gets an unmatchable prefix, not the
                // bare `{toolkit}:` — that widened prefix matched *every*
                // connection of the toolkit, so a malformed or legacy Gmail
                // source reported another Gmail connection's ingest counts as
                // its own. Chunk ids are `{toolkit}:{connection_id}:{doc}`, so
                // an entry with no connection id has no rows it could name;
                // the driver's zero-fill guarantee turns the non-match into an
                // idle row. Same sentinel style as `__no_toolkit__` below.
                (Some(toolkit), None) => format!("{toolkit}:__no_connection__:"),
                (None, _) => "__no_toolkit__:".to_string(),
            }
        }
        _ => format!("mem_src:{}:", source.id),
    }
}

/// Status for every configured source, in registry order.
///
/// One driver call for the whole batch where the engine ran one SQL round trip
/// per source. That is the shape the contract member is built for — it echoes
/// [`SourceIngestQuery::source_id`] on every row so the pairing is by value
/// rather than by position — and it removes the per-source connection open the
/// old loop paid.
///
/// **Every configured source gets a row**, including one that has never synced:
/// the driver zero-fills a prefix that matches nothing, which is the whole
/// reason this is not `source_totals`. Disabled sources are included too,
/// exactly as the engine's loop included them — the panel renders them and
/// filtering them here would empty rows the user can still see.
///
/// # Errors
///
/// Only when the source registry itself cannot be read. A driver with no chunk
/// tier, and a read that fails, both degrade to a zero row per source with a
/// warning — the engine's loop degraded the same way, per source, and a status
/// surface that errors tells the user less than one reporting an idle store.
pub async fn status_list(config: &Config) -> Result<Vec<SourceStatus>, String> {
    let sources = super::registry::list_sources().await?;
    if sources.is_empty() {
        return Ok(Vec::new());
    }

    let queries: Vec<SourceIngestQuery> = sources
        .iter()
        .map(|source| SourceIngestQuery {
            source_id: source.id.clone(),
            chunk_id_prefix: source_id_prefix(source),
        })
        .collect();

    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let rows = ingest_rows(&binding, &queries).await;

    // The clock is read once for the whole batch, so two sources synced in the
    // same instant cannot be labelled differently by the time the loop reached
    // the second one.
    let now_ms = chrono::Utc::now().timestamp_millis();
    Ok(sources
        .into_iter()
        .map(|source| {
            // Paired by `source_id`, not by position: the member echoes the id
            // precisely so a caller need not trust the order, and a mis-indexed
            // row here would report one connector's progress under another's
            // name.
            match rows.iter().find(|row| row.source_id == source.id) {
                Some(row) => SourceStatus {
                    source_id: source.id,
                    chunks_synced: row.chunks_synced,
                    chunks_pending: row.chunks_pending,
                    last_chunk_at_ms: row.last_chunk_at_ms,
                    freshness: FreshnessLabel::from_age_ms(row.last_chunk_at_ms, now_ms),
                },
                None => SourceStatus::idle(source.id),
            }
        })
        .collect())
}

/// The driver's answer for one batch of queries, degraded to no rows on any
/// failure.
///
/// Split out so [`status_list`] reads as the mapping it is. Returning an empty
/// vector rather than an error is what turns every source into
/// [`SourceStatus::idle`] above, which is the engine loop's own degrade.
async fn ingest_rows(
    binding: &MemoryBinding,
    queries: &[SourceIngestQuery],
) -> Vec<crate::openhuman::memory::api::provider::chunks::SourceIngestStatus> {
    let Some(chunks) = binding.provider().as_chunks() else {
        tracing::warn!(
            driver = %binding.driver_id(),
            "[memory_sources:status] driver does not serve Chunks; reporting every source idle"
        );
        return Vec::new();
    };
    match chunks.source_ingest_status(queries).await {
        Ok(rows) => rows,
        Err(error) => {
            // The batch fails as one — the counts come from a single store, so
            // a read that fails fails for all of them — where the engine failed
            // per source. Same outcome for the reader either way: a zero row.
            tracing::warn!(
                error = %error,
                sources = queries.len(),
                "[memory_sources:status] ingest status read failed"
            );
            Vec::new()
        }
    }
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
