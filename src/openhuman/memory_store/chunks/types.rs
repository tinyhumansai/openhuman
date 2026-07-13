//! Core types for the memory tree ingestion layer (Phase 1 / issue #707).
//!
//! This module defines the canonical [`Chunk`] representation produced by the
//! ingestion pipeline along with its provenance [`Metadata`] and back-pointer
//! [`SourceRef`]. These types feed into later phases (#708 scoring, #709
//! summary trees, #710 retrieval) but are self-contained at Phase 1.
//!
//! All chunk IDs are deterministic: `sha256(source_kind | "\0" | source_id |
//! "\0" | seq | "\0" | content)` truncated to 32 hex chars so re-ingest of the
//! same source material yields stable IDs and idempotent upserts.
//!
//! **W3 type cutover:** these types + chunk-id/token helpers are now
//! **re-exported from the `tinycortex` crate** (ported from this exact module —
//! identical fields, derives, serde wire form, and `chunk_id` derivation, all
//! pinned by the tests below). Re-exporting keeps one source of truth and lets
//! the chunk store operations delegate to the crate without host↔crate type
//! conversions. `StagedChunk` (in `memory_store::content`) and `DataSource` stay
//! host for now — `DataSource` is a provider taxonomy the chunk *store* never
//! touches (it's used by ingest canonicalization + scoring), and re-exporting it
//! would force `_` arms on the host's exhaustive matches of a now-foreign
//! `#[non_exhaustive]` enum. It flips with the ingest module (W6).

pub use tinycortex::memory::chunks::{
    approx_token_count, chunk_id, conservative_token_estimate, truncate_to_conservative_tokens,
    Chunk, DataSource, Metadata, SourceKind, SourceRef,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_id_is_deterministic() {
        let a = chunk_id(SourceKind::Chat, "slack:#eng", 0, "hello");
        let b = chunk_id(SourceKind::Chat, "slack:#eng", 0, "hello");
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn conservative_estimate_weights_by_char_class() {
        assert_eq!(conservative_token_estimate("abcd"), 2); // 4 alnum × 2q / 4
        assert_eq!(conservative_token_estimate("    "), 1); // 4 ws × 1q / 4
        assert_eq!(conservative_token_estimate("....,,,,"), 8); // 8 punct × 4q / 4
        assert_eq!(conservative_token_estimate("שלום"), 4); // 4 non-ascii × 4q / 4
        assert_eq!(conservative_token_estimate(""), 0);
    }

    #[test]
    fn conservative_estimate_exceeds_approx_for_dense_content() {
        // Hash/path/punctuation-dense ASCII — exactly the content that defeated
        // the chars/4 heuristic and overflowed bge-m3.
        let dense =
            "claude-memory:openhuman:MEMORY.md:67d6fe2727d431b16d41630babfdcf1cdf61bda7b9ba\n"
                .repeat(40);
        assert!(
            conservative_token_estimate(&dense) > approx_token_count(&dense),
            "conservative estimate must exceed chars/4 on dense content",
        );
    }

    #[test]
    fn truncate_respects_budget_and_char_boundaries() {
        let text = "שלום עולם ".repeat(100); // Hebrew, ~1 token/char
        let out = truncate_to_conservative_tokens(&text, 10);
        assert!(conservative_token_estimate(out) <= 10);
        assert!(text.starts_with(out)); // valid prefix on a char boundary
        assert!(out.len() < text.len());
    }

    #[test]
    fn truncate_is_noop_within_budget() {
        let text = "short and sweet";
        assert_eq!(truncate_to_conservative_tokens(text, 1000), text);
    }

    #[test]
    fn chunk_id_varies_with_seq() {
        let a = chunk_id(SourceKind::Chat, "slack:#eng", 0, "hello");
        let b = chunk_id(SourceKind::Chat, "slack:#eng", 1, "hello");
        assert_ne!(a, b);
    }

    #[test]
    fn chunk_id_varies_with_source_kind() {
        let a = chunk_id(SourceKind::Chat, "foo", 0, "hello");
        let b = chunk_id(SourceKind::Email, "foo", 0, "hello");
        assert_ne!(a, b);
    }

    #[test]
    fn chunk_id_varies_with_source_id() {
        let a = chunk_id(SourceKind::Chat, "x", 0, "hello");
        let b = chunk_id(SourceKind::Chat, "y", 0, "hello");
        assert_ne!(a, b);
    }

    #[test]
    fn chunk_id_varies_with_content() {
        // Critical for the per-connection source_id design: two ingests
        // sharing source_id but different content (e.g. different 6-hour
        // Slack buckets) must produce distinct ids at seq=0,1,2,…
        let a = chunk_id(SourceKind::Chat, "slack:c1", 0, "bucket A content");
        let b = chunk_id(SourceKind::Chat, "slack:c1", 0, "bucket B content");
        assert_ne!(a, b);
    }

    #[test]
    fn source_kind_round_trip() {
        for kind in [SourceKind::Chat, SourceKind::Email, SourceKind::Document] {
            assert_eq!(SourceKind::parse(kind.as_str()).unwrap(), kind);
        }
    }

    #[test]
    fn data_source_round_trip() {
        for ds in DataSource::all() {
            assert_eq!(DataSource::parse(ds.as_str()).unwrap(), *ds);
        }
    }

    #[test]
    fn data_source_has_all_variants() {
        assert_eq!(DataSource::all().len(), 9);
    }

    #[test]
    fn data_source_kind_mapping() {
        use DataSource::*;
        for ds in [Discord, Telegram, Whatsapp, Conversation] {
            assert_eq!(ds.kind(), SourceKind::Chat);
        }
        for ds in [Gmail, OtherEmail] {
            assert_eq!(ds.kind(), SourceKind::Email);
        }
        for ds in [Notion, MeetingNotes, DriveDocs] {
            assert_eq!(ds.kind(), SourceKind::Document);
        }
    }

    #[test]
    fn data_source_parse_rejects_unknown() {
        assert!(DataSource::parse("nope").is_err());
        // Ensure our snake_case wire form is exactly what callers send.
        assert!(DataSource::parse("Discord").is_err()); // case-sensitive
        assert!(DataSource::parse("drive docs").is_err()); // no spaces
    }

    #[test]
    fn data_source_serde_is_snake_case() {
        let ds = DataSource::MeetingNotes;
        let json = serde_json::to_string(&ds).unwrap();
        assert_eq!(json, "\"meeting_notes\"");
        let parsed: DataSource = serde_json::from_str("\"meeting_notes\"").unwrap();
        assert_eq!(parsed, ds);
    }

    #[test]
    fn approx_token_count_scales_linearly() {
        assert_eq!(approx_token_count(""), 0);
        assert_eq!(approx_token_count("a"), 1); // 1→1
        assert_eq!(approx_token_count("abcd"), 1); // 4→1
        assert_eq!(approx_token_count("abcde"), 2); // 5→2
        assert_eq!(approx_token_count(&"x".repeat(400)), 100);
    }
}
