//! Per-source sync status — chunks ingested, freshness, in-flight progress.
//!
//! Queries `mem_tree_chunks` filtered by source-id prefix:
//! - Reader-backed kinds (folder/github/rss/web/twitter) tag chunks
//!   with `mem_src:{source.id}:%`, so we count those directly.
//! - Composio sources tag chunks with the toolkit-specific id
//!   (e.g. `gmail:user@example.com:msg_xxx`), so we match by toolkit
//!   prefix instead.

use serde::Serialize;

use crate::openhuman::config::Config;
use crate::openhuman::memory_sources::types::{MemorySourceEntry, SourceKind};
use crate::openhuman::memory_store::chunks::store::with_connection;

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
        with_connection(&cfg, |conn| {
            let prefix = source_id_prefix(&source_clone);

            // Surface real query errors so status telemetry doesn't lie about
            // a healthy zero-row state when the DB is actually broken.
            let (synced, pending, last_ts): (i64, i64, Option<i64>) = conn.query_row(
                "SELECT \
                       COUNT(*), \
                       SUM(CASE WHEN embedding IS NULL THEN 1 ELSE 0 END), \
                       MAX(timestamp_ms) \
                     FROM mem_tree_chunks \
                     WHERE source_id LIKE ?1",
                [&prefix],
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
    let sources = crate::openhuman::memory_sources::registry::list_sources().await?;
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
    use super::*;

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
        let mut entry = MemorySourceEntry {
            id: "src_abc".into(),
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
            selector: None,
        };
        assert_eq!(source_id_prefix(&entry), "mem_src:src_abc:%");

        entry.kind = SourceKind::Composio;
        entry.toolkit = Some("gmail".into());
        assert_eq!(source_id_prefix(&entry), "gmail:%");
    }
}
