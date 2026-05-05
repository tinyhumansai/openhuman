//! Memory sync status — types (#1136, simplified rewrite).
//!
//! The original implementation tracked phase + counters via push-based
//! events from each provider's sync loop. That was racy, lied about
//! "downloading 0/0" while work was in flight, and required maintaining
//! a parallel KV store. Replaced with a pull model: count chunks in
//! `mem_tree_chunks` GROUPED BY source_kind on each RPC. The chunks
//! table is the source of truth — if a chunk exists, that source has
//! synced something; the count is exact at any moment.
//!
//! Activity-freshness is derived from `MAX(timestamp_ms)` per group.

use serde::Serialize;

/// User-facing label derived from how recently chunks were ingested.
///
/// Computed at RPC time, not stored. Boundaries are deliberate:
/// `Active` matches "currently syncing" (a fresh chunk in the last 30s
/// suggests live ingest), `Recent` covers "synced this session", and
/// `Idle` is everything older.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessLabel {
    Active,
    Recent,
    Idle,
}

impl FreshnessLabel {
    /// Map `last_chunk_at_ms` to a label using `now_ms` as reference.
    /// Returns `Idle` when `last_chunk_at_ms` is `None`.
    pub fn from_age_ms(last_chunk_at_ms: Option<i64>, now_ms: i64) -> Self {
        match last_chunk_at_ms {
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

/// One row per provider (slack/gmail/discord/notion/…) that has
/// produced chunks. The provider name is parsed from each chunk's
/// `source_id` prefix (everything before the first `:`).
#[derive(Clone, Debug, Serialize)]
pub struct MemorySyncStatus {
    /// Specific provider — `"slack"`, `"gmail"`, `"discord"`,
    /// `"telegram"`, `"whatsapp"`, `"notion"`, `"meeting_notes"`,
    /// `"drive_docs"`, etc. Derived from `source_id` prefix; falls
    /// back to the broad `source_kind` category for chunks whose
    /// `source_id` has no `:` separator.
    pub provider: String,
    /// Total chunks in `mem_tree_chunks` for this source_kind.
    pub chunks_synced: u64,
    /// Chunks fetched + stored but not yet processed by the extract+embed
    /// background worker (`embedding IS NULL`). Lifetime metric — counts
    /// every still-pending chunk regardless of when it was ingested.
    pub chunks_pending: u64,
    /// Total chunks in the *current sync wave* — i.e., chunks created
    /// at-or-after the oldest currently-pending chunk's `created_at_ms`.
    /// When `chunks_pending == 0` this is also 0 (no active wave).
    pub batch_total: u64,
    /// Of `batch_total`, how many have been processed (`embedding IS NOT
    /// NULL`) since the wave started. Progress fill = `batch_processed /
    /// batch_total`.
    pub batch_processed: u64,
    /// Most recent chunk's `timestamp_ms` for this source_kind, or
    /// `None` if no chunks yet.
    pub last_chunk_at_ms: Option<i64>,
    /// Derived from `last_chunk_at_ms` at RPC time.
    pub freshness: FreshnessLabel,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freshness_label_active_within_30s() {
        let now = 1_777_000_000_000;
        assert_eq!(
            FreshnessLabel::from_age_ms(Some(now - 1_000), now),
            FreshnessLabel::Active
        );
        assert_eq!(
            FreshnessLabel::from_age_ms(Some(now - 29_999), now),
            FreshnessLabel::Active
        );
    }

    #[test]
    fn freshness_label_recent_between_30s_and_5min() {
        let now = 1_777_000_000_000;
        assert_eq!(
            FreshnessLabel::from_age_ms(Some(now - 30_001), now),
            FreshnessLabel::Recent
        );
        assert_eq!(
            FreshnessLabel::from_age_ms(Some(now - 4 * 60_000), now),
            FreshnessLabel::Recent
        );
    }

    #[test]
    fn freshness_label_idle_beyond_5min() {
        let now = 1_777_000_000_000;
        assert_eq!(
            FreshnessLabel::from_age_ms(Some(now - 5 * 60_000 - 1), now),
            FreshnessLabel::Idle
        );
        assert_eq!(FreshnessLabel::from_age_ms(None, now), FreshnessLabel::Idle);
    }
}
