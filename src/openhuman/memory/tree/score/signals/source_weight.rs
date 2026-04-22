//! Source-weight signal — per-provider base weight derived from the
//! `DataSource` when it can be inferred from a chunk's tags.
//!
//! Rationale from `Memory Architecture.md` (Step 2.3 "Source scoring"):
//! - High-intentionality messaging (direct DMs, personal emails) scores higher
//! - Broadcast/channel content scores lower
//! - Documents authored by the user score higher than shared-but-unmodified drops
//!
//! Phase 2 takes a conservative approach: per-[`DataSource`] base weight.
//! Finer distinction (DM vs channel on Slack specifically) requires richer
//! ingest-time metadata and is deferred.

use crate::openhuman::memory::tree::types::{DataSource, Metadata, SourceKind};

const PROVIDER_PREFIX: &str = "provider:";

/// Best-effort map from `Metadata` to a [`DataSource`] — checks the `tags`
/// list for a stable `provider:<snake_case>` provider tag. If not present,
/// falls back to kind-based defaults.
///
/// The ingestion pipeline can (and should) add a provider tag on the
/// canonicalised output so this signal fires deterministically. Until that's
/// wired everywhere, we fall back to the kind-level default.
pub fn infer_data_source(meta: &Metadata) -> Option<DataSource> {
    for tag in &meta.tags {
        let Some(provider) = tag.strip_prefix(PROVIDER_PREFIX) else {
            continue;
        };
        if let Ok(ds) = DataSource::parse(provider) {
            return Some(ds);
        }
    }
    None
}

/// Score in `[0.0, 1.0]` for the chunk's originating provider.
pub fn score(meta: &Metadata) -> f32 {
    if let Some(ds) = infer_data_source(meta) {
        return weight_for(ds);
    }
    // Fallback: kind-level defaults consistent with per-provider averages.
    match meta.source_kind {
        SourceKind::Email => 0.75,
        SourceKind::Document => 0.7,
        SourceKind::Chat => 0.5,
    }
}

fn weight_for(ds: DataSource) -> f32 {
    match ds {
        // Personal email providers score high — typically small, directed audiences
        DataSource::Gmail => 0.8,
        DataSource::OtherEmail => 0.7,
        // Chat providers differ: WhatsApp is typically DM-heavy, Discord
        // can be broadcast-heavy, Telegram mixes both
        DataSource::Whatsapp => 0.75,
        DataSource::Telegram => 0.6,
        DataSource::Discord => 0.5,
        // Documents: Notion = structured, Drive = mixed, Meeting notes = high value
        DataSource::Notion => 0.75,
        DataSource::DriveDocs => 0.6,
        DataSource::MeetingNotes => 0.85,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn meta_with_tag(kind: SourceKind, tag: &str) -> Metadata {
        let mut m = Metadata::point_in_time(kind, "x", "owner", Utc::now());
        m.tags.push(tag.to_string());
        m
    }

    #[test]
    fn data_source_inferred_from_tags() {
        let m = meta_with_tag(SourceKind::Chat, "provider:whatsapp");
        assert_eq!(infer_data_source(&m), Some(DataSource::Whatsapp));
    }

    #[test]
    fn plain_user_label_does_not_infer_provider() {
        let m = meta_with_tag(SourceKind::Email, "notion");
        assert_eq!(infer_data_source(&m), None);
        assert!((score(&m) - 0.75).abs() < 1e-6);
    }

    #[test]
    fn unknown_tag_falls_back_to_kind_default() {
        let m = meta_with_tag(SourceKind::Email, "not-a-data-source");
        let s = score(&m);
        assert!((s - 0.75).abs() < 1e-6);
    }

    #[test]
    fn provider_specific_weights_applied() {
        let m = meta_with_tag(SourceKind::Document, "provider:meeting_notes");
        assert!((score(&m) - 0.85).abs() < 1e-6);
    }

    #[test]
    fn all_data_sources_bounded() {
        for ds in DataSource::all() {
            let w = weight_for(*ds);
            assert!((0.0..=1.0).contains(&w));
        }
    }
}
