//! Metadata-weight signal — base weight from the source kind's grouping.
//!
//! The idea: a 1:1 email thread is inherently higher-signal than a broadcast
//! Slack channel, regardless of content. This signal captures the "shape"
//! of the interaction: how scoped is the audience?
//!
//! Phase 2 keeps this simple: one weight per `SourceKind`. Per-grouping
//! context (e.g., channel size, thread participant count) is a future
//! refinement when we actually have that metadata at ingest.

use crate::openhuman::memory::tree::types::{Metadata, SourceKind};

/// Base weight for each source kind.
///
/// Email threads are typically scoped (1:1 or small groups, directed).
/// Documents are single-author outputs — high intentionality per chunk.
/// Chats vary widely; base weight is lower because the channel could be
/// a 200-person broadcast or a tight DM — the interaction signal disambiguates.
pub fn score(meta: &Metadata) -> f32 {
    match meta.source_kind {
        SourceKind::Email => 0.8,
        SourceKind::Document => 0.9,
        SourceKind::Chat => 0.5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn meta(kind: SourceKind) -> Metadata {
        Metadata::point_in_time(kind, "x", "owner", Utc::now())
    }

    #[test]
    fn per_kind_weights() {
        assert!(score(&meta(SourceKind::Document)) > score(&meta(SourceKind::Email)));
        assert!(score(&meta(SourceKind::Email)) > score(&meta(SourceKind::Chat)));
    }

    #[test]
    fn bounded_zero_one() {
        for k in [SourceKind::Chat, SourceKind::Email, SourceKind::Document] {
            let s = score(&meta(k));
            assert!((0.0..=1.0).contains(&s));
        }
    }
}
