//! Per-source sync status — chunks ingested, freshness, in-flight progress.
//!
//! Queries `mem_tree_chunks` filtered by source-id prefix:
//! - Reader-backed kinds (folder/github/rss/web/twitter) tag chunks
//!   with `mem_src:{source.id}:%`, so we count those directly.
//! - Composio sources tag chunks with the toolkit-specific id
//!   (e.g. `gmail:user@example.com:msg_xxx`), so we match by toolkit
//!   prefix instead.
//!
//! "Pending" means *not yet resolved for the active embedding signature*: the
//! chunk has no vector in the `mem_tree_chunk_embeddings` sidecar under
//! [`tree_active_signature`], no re-embed tombstone for that signature, and was
//! not dropped by the admission gate. This mirrors the resolution rule the
//! provider-level sibling (`tinycortex::memory::sync::list_sync_statuses`) and
//! `has_uncovered_reembed_work` already use, so a settled store reports zero.

use serde::Serialize;

use crate::openhuman::config::Config;
use crate::openhuman::memory::sources::types::{MemorySourceEntry, SourceKind};
use crate::openhuman::memory::store::chunks::store::{
    tree_active_signature, with_connection, CHUNK_STATUS_DROPPED,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessLabel {
    Active,
    Recent,
    Idle,
}

impl FreshnessLabel {
    pub fn from_age_ms(last_ms: Option<i64>, now_ms: i64) -> Self {
        match last_ms {
            None => Self::Idle,
            Some(ts) => {
                let age = now_ms.saturating_sub(ts);
                if age <= 30_000 {
                    Self::Active
                } else if age <= 5 * 60_000 {
                    Self::Recent
                } else {
                    Self::Idle
                }
            }
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceStatus {
    pub source_id: String,
    pub chunks_synced: u64,
    pub chunks_pending: u64,
    pub last_chunk_at_ms: Option<i64>,
    pub freshness: FreshnessLabel,
}

/// Compute status for one source.
pub async fn source_status(
    config: &Config,
    source: &MemorySourceEntry,
) -> Result<SourceStatus, String> {
    let cfg = config.clone();
    let source_clone = source.clone();

    tokio::task::spawn_blocking(move || {
        // Embeddings are scoped per (chunk, model signature); a vector stored
        // under a superseded signature is unreachable for the active vector
        // space, so it must still read as pending.
        let signature = tree_active_signature(&cfg);

        with_connection(&cfg, |conn| {
            let prefix = source_id_prefix(&source_clone);

            // Surface real query errors so status telemetry doesn't lie about
            // a healthy zero-row state when the DB is actually broken.
            let (synced, pending, last_ts): (i64, i64, Option<i64>) = conn.query_row(
                "SELECT \
                       COUNT(*), \
                       SUM(CASE WHEN EXISTS ( \
                                    SELECT 1 FROM mem_tree_chunk_embeddings e \
                                     WHERE e.chunk_id = c.id \
                                       AND e.model_signature = ?2) \
                                 OR EXISTS ( \
                                    SELECT 1 FROM mem_tree_chunk_reembed_skipped s \
                                     WHERE s.chunk_id = c.id \
                                       AND s.model_signature = ?2) \
                                 OR c.lifecycle_status = ?3 \
                                THEN 0 ELSE 1 END), \
                       MAX(c.timestamp_ms) \
                     FROM mem_tree_chunks c \
                     WHERE c.source_id LIKE ?1",
                rusqlite::params![prefix, signature, CHUNK_STATUS_DROPPED],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                        r.get(2)?,
                    ))
                },
            )?;

            let now_ms = chrono::Utc::now().timestamp_millis();
            Ok(SourceStatus {
                source_id: source_clone.id.clone(),
                chunks_synced: synced.max(0) as u64,
                chunks_pending: pending.max(0) as u64,
                last_chunk_at_ms: last_ts,
                freshness: FreshnessLabel::from_age_ms(last_ts, now_ms),
            })
        })
        .map_err(|e| format!("source_status: {e}"))
    })
    .await
    .map_err(|e| format!("source_status join: {e}"))?
}

/// Compute status for all configured sources (one SQL roundtrip per source).
pub async fn status_list(config: &Config) -> Result<Vec<SourceStatus>, String> {
    let sources = crate::openhuman::memory::sources::registry::list_sources().await?;
    let mut out = Vec::with_capacity(sources.len());
    for source in sources {
        match source_status(config, &source).await {
            Ok(s) => out.push(s),
            Err(e) => {
                tracing::warn!(
                    source_id = %source.id,
                    error = %e,
                    "[memory_sources:status] query failed"
                );
                out.push(SourceStatus {
                    source_id: source.id,
                    chunks_synced: 0,
                    chunks_pending: 0,
                    last_chunk_at_ms: None,
                    freshness: FreshnessLabel::Idle,
                });
            }
        }
    }
    Ok(out)
}

/// Build the `source_id LIKE` prefix that matches chunks belonging to a source.
fn source_id_prefix(source: &MemorySourceEntry) -> String {
    match source.kind {
        SourceKind::Composio => {
            // Composio providers write chunks with source_id = `{toolkit}:%`
            // (e.g. `gmail:user@example.com:msg_xxx`). Match by toolkit only.
            source
                .toolkit
                .as_deref()
                .map(|t| format!("{t}:%"))
                .unwrap_or_else(|| "__no_toolkit__:%".to_string())
        }
        _ => format!("mem_src:{}:%", source.id),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;

    use super::*;
    use crate::openhuman::memory::store::chunks::store::{
        mark_chunk_reembed_skipped, set_chunk_embedding_for_signature, set_chunk_lifecycle_status,
        upsert_chunks,
    };
    use crate::openhuman::memory::store::chunks::types::{
        chunk_id, Chunk, Metadata, SourceKind as ChunkSourceKind,
    };

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    fn source_entry(id: &str) -> MemorySourceEntry {
        MemorySourceEntry {
            id: id.into(),
            kind: SourceKind::Folder,
            label: "x".into(),
            enabled: true,
            toolkit: None,
            connection_id: None,
            path: Some("/tmp".into()),
            glob: None,
            url: None,
            branch: None,
            paths: Vec::new(),
            query: None,
            since_days: None,
            max_items: None,
            max_commits: None,
            max_issues: None,
            max_prs: None,
            selector: None,
            max_tokens_per_sync: None,
            max_cost_per_sync_usd: None,
            sync_depth_days: None,
        }
    }

    fn chunk(source_id: &str, seq: u32, timestamp_ms: i64) -> Chunk {
        let ts = Utc.timestamp_millis_opt(timestamp_ms).unwrap();
        let content = format!("status chunk {source_id} #{seq}");
        Chunk {
            id: chunk_id(ChunkSourceKind::Document, source_id, seq, &content),
            content,
            metadata: Metadata::point_in_time(ChunkSourceKind::Document, source_id, "test", ts),
            token_count: 1,
            seq_in_source: seq,
            created_at: ts,
            partial_message: false,
        }
    }

    fn seed(cfg: &Config, source: &str, count: u32) -> Vec<Chunk> {
        let mut chunks = Vec::new();
        for seq in 0..count {
            let source_id = format!("mem_src:{source}:item-{seq}");
            chunks.push(chunk(&source_id, seq, 1_700_000_000_000 + i64::from(seq)));
        }
        upsert_chunks(cfg, &chunks).unwrap();
        chunks
    }

    async fn status_of(cfg: &Config, id: &str) -> SourceStatus {
        source_status(cfg, &source_entry(id)).await.unwrap()
    }

    #[test]
    fn freshness_thresholds() {
        let now = 1_000_000_000_000;
        assert_eq!(
            FreshnessLabel::from_age_ms(Some(now - 1_000), now),
            FreshnessLabel::Active
        );
        assert_eq!(
            FreshnessLabel::from_age_ms(Some(now - 60_000), now),
            FreshnessLabel::Recent
        );
        assert_eq!(
            FreshnessLabel::from_age_ms(Some(now - 600_000), now),
            FreshnessLabel::Idle
        );
        assert_eq!(FreshnessLabel::from_age_ms(None, now), FreshnessLabel::Idle);
    }

    #[test]
    fn source_id_prefix_dispatch() {
        let mut entry = source_entry("src_abc");
        assert_eq!(source_id_prefix(&entry), "mem_src:src_abc:%");

        entry.kind = SourceKind::Composio;
        entry.toolkit = Some("gmail".into());
        assert_eq!(source_id_prefix(&entry), "gmail:%");
    }

    /// The reported bug (#5329): the old query read the vestigial
    /// `mem_tree_chunks.embedding` column, which no production writer ever
    /// populates, so `pending` always equalled `synced`. Reading the sidecar
    /// lets a fully-embedded source settle at zero.
    #[tokio::test]
    async fn pending_reaches_zero_once_every_chunk_has_an_active_embedding() {
        let (_tmp, cfg) = test_config();
        let chunks = seed(&cfg, "src_done", 2);
        let active = tree_active_signature(&cfg);
        for c in &chunks {
            set_chunk_embedding_for_signature(&cfg, &c.id, &active, &[0.5]).unwrap();
        }

        let status = status_of(&cfg, "src_done").await;
        assert_eq!(status.chunks_synced, 2);
        assert_eq!(
            status.chunks_pending, 0,
            "pre-fix this reported pending == synced"
        );
    }

    /// A vector stored under a superseded model signature does not make the
    /// chunk reachable in the active vector space, so it must stay pending.
    #[tokio::test]
    async fn pending_ignores_embeddings_from_a_superseded_signature() {
        let (_tmp, cfg) = test_config();
        let chunks = seed(&cfg, "src_mixed", 3);
        let active = tree_active_signature(&cfg);
        set_chunk_embedding_for_signature(&cfg, &chunks[0].id, &active, &[0.1, 0.2]).unwrap();
        set_chunk_embedding_for_signature(&cfg, &chunks[1].id, "stale/model@7", &[0.3]).unwrap();

        let status = status_of(&cfg, "src_mixed").await;
        assert_eq!(status.chunks_synced, 3);
        assert_eq!(
            status.chunks_pending, 2,
            "only the active-signature vector resolves a chunk"
        );
    }

    /// Chunks that will never be embedded — a re-embed tombstone for the active
    /// signature, or a chunk the admission gate dropped — must not be counted,
    /// otherwise the counter can never drain. Matches the resolution rule in
    /// `tinycortex::memory::sync::list_sync_statuses`.
    #[tokio::test]
    async fn pending_excludes_tombstoned_and_dropped_chunks() {
        let (_tmp, cfg) = test_config();
        let chunks = seed(&cfg, "src_terminal", 3);
        let active = tree_active_signature(&cfg);
        mark_chunk_reembed_skipped(&cfg, &chunks[0].id, &active, "too large").unwrap();
        set_chunk_lifecycle_status(&cfg, &chunks[1].id, CHUNK_STATUS_DROPPED).unwrap();

        let status = status_of(&cfg, "src_terminal").await;
        assert_eq!(status.chunks_synced, 3);
        assert_eq!(status.chunks_pending, 1, "only the untouched chunk pends");
    }

    /// A source with no chunks at all must report zeroes rather than erroring
    /// on the `SUM`/`MAX` NULLs an empty scan produces.
    #[tokio::test]
    async fn empty_source_reports_zeroed_status() {
        let (_tmp, cfg) = test_config();
        let status = status_of(&cfg, "src_empty").await;
        assert_eq!(status.chunks_synced, 0);
        assert_eq!(status.chunks_pending, 0);
        assert_eq!(status.last_chunk_at_ms, None);
        assert_eq!(status.freshness, FreshnessLabel::Idle);
    }
}
