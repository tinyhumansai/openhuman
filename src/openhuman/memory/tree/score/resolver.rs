//! Entity canonicalisation / cross-platform merge (Phase 2 / #708, V1).
//!
//! Exact-match only: normalises surface forms (lowercase emails, strip
//! leading `@` on handles) and assigns a canonical `entity_id` string.
//!
//! Fuzzy matching (alice-slack ≡ Alice-Discord by soft match) is deferred
//! until we have real entity-graph data — the current implementation
//! handles the mechanical cases cleanly without producing false merges.

use serde::{Deserialize, Serialize};

use crate::openhuman::memory::tree::score::extract::{EntityKind, ExtractedEntities};

/// Canonicalised entity — same shape as [`ExtractedEntity`] plus a stable
/// `canonical_id` suitable for indexing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanonicalEntity {
    pub canonical_id: String,
    pub kind: EntityKind,
    pub surface: String,
    pub span_start: u32,
    pub span_end: u32,
    pub score: f32,
}

/// Canonicalise a batch of extracted entities.
///
/// Same surface form (after normalisation) → same `canonical_id` regardless
/// of how many times it appears in a chunk. Preserves source spans by
/// emitting one [`CanonicalEntity`] per occurrence.
///
/// Extracted **topics** are also promoted into the canonical stream under
/// [`EntityKind::Topic`] so downstream routing (Phase 3c topic trees) can
/// treat themes as first-class scope alongside people/orgs. Topics have no
/// source span (they're derived from the whole chunk, not a specific
/// substring), so `span_start` / `span_end` are both `0` for topic rows —
/// readers should key on `kind` instead of span when span-awareness matters.
pub fn canonicalise(extracted: &ExtractedEntities) -> Vec<CanonicalEntity> {
    let mut out: Vec<CanonicalEntity> = extracted
        .entities
        .iter()
        .map(|e| CanonicalEntity {
            canonical_id: canonical_id_for(e.kind, &e.text),
            kind: e.kind,
            surface: e.text.clone(),
            span_start: e.span_start,
            span_end: e.span_end,
            score: e.score,
        })
        .collect();

    // Promote topics. Dedup against the entities we already emitted so a
    // hashtag like `#launch` and a topic label `"launch"` don't both land
    // as the same canonical id with the same kind — the hashtag keeps its
    // Hashtag kind, the topic gets Topic kind, and `canonical_id_for`
    // makes them distinguishable: `hashtag:launch` vs `topic:launch`.
    for topic in &extracted.topics {
        let canonical_id = canonical_id_for(EntityKind::Topic, &topic.label);
        // Dedup within the topic set in case the scorer produces the same
        // label twice (LLM + regex overlap). Entities under other kinds
        // aren't dedup targets — `topic:launch` and `hashtag:launch` are
        // intentionally separate.
        if out
            .iter()
            .any(|e| e.kind == EntityKind::Topic && e.canonical_id == canonical_id)
        {
            continue;
        }
        out.push(CanonicalEntity {
            canonical_id,
            kind: EntityKind::Topic,
            surface: topic.label.clone(),
            span_start: 0,
            span_end: 0,
            score: topic.score,
        });
    }
    out
}

/// Canonical id form per kind. Deterministic so the same surface always
/// maps to the same id.
///
/// - Email: `email:lowercased`
/// - Handle: `handle:lowercased` with leading `@` stripped
/// - Hashtag: `hashtag:lowercased` with leading `#` stripped
/// - URL: `url:trimmed` with case preserved for path/query exact matching
/// - Semantic kinds: `kind:lowercased-surface` (V1; fuzzy merge deferred)
pub fn canonical_id_for(kind: EntityKind, surface: &str) -> String {
    let trimmed = surface.trim();
    let clean = if kind == EntityKind::Url {
        trimmed.to_string()
    } else {
        trimmed
            .to_lowercase()
            .trim_start_matches('@')
            .trim_start_matches('#')
            .to_string()
    };
    format!("{}:{}", kind.as_str(), clean)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::score::extract::ExtractedEntity;

    fn entity(kind: EntityKind, text: &str) -> ExtractedEntity {
        ExtractedEntity {
            kind,
            text: text.to_string(),
            span_start: 0,
            span_end: text.chars().count() as u32,
            score: 1.0,
        }
    }

    #[test]
    fn email_case_insensitive_canonicalises() {
        let a = canonical_id_for(EntityKind::Email, "Alice@Example.com");
        let b = canonical_id_for(EntityKind::Email, "alice@example.com");
        assert_eq!(a, b);
        assert_eq!(a, "email:alice@example.com");
    }

    #[test]
    fn handle_strips_leading_at() {
        let a = canonical_id_for(EntityKind::Handle, "@alice");
        let b = canonical_id_for(EntityKind::Handle, "alice");
        assert_eq!(a, b);
        assert_eq!(a, "handle:alice");
    }

    #[test]
    fn hashtag_strips_leading_hash() {
        let a = canonical_id_for(EntityKind::Hashtag, "#launch");
        let b = canonical_id_for(EntityKind::Hashtag, "launch");
        assert_eq!(a, b);
    }

    #[test]
    fn url_preserves_case() {
        let id = canonical_id_for(EntityKind::Url, " https://example.com/Path?Token=ABC ");
        assert_eq!(id, "url:https://example.com/Path?Token=ABC");
    }

    #[test]
    fn canonicalise_batch_preserves_spans() {
        let ex = ExtractedEntities {
            entities: vec![
                entity(EntityKind::Email, "Alice@Example.com"),
                entity(EntityKind::Email, "alice@example.com"),
            ],
            topics: vec![],
            llm_importance: None,
            llm_importance_reason: None,
        };
        let out = canonicalise(&ex);
        assert_eq!(out.len(), 2);
        // Both map to the same canonical id (merge-equivalent)
        assert_eq!(out[0].canonical_id, out[1].canonical_id);
        // But surface forms remain distinct
        assert_ne!(out[0].surface, out[1].surface);
    }

    #[test]
    fn different_kinds_produce_different_ids_for_same_text() {
        assert_ne!(
            canonical_id_for(EntityKind::Handle, "alice"),
            canonical_id_for(EntityKind::Person, "alice")
        );
    }

    // ── Topic canonicalisation (#709 / Phase 3c topic-tree scope) ────

    use crate::openhuman::memory::tree::score::extract::ExtractedTopic;

    fn topic(label: &str, score: f32) -> ExtractedTopic {
        ExtractedTopic {
            label: label.to_string(),
            score,
        }
    }

    #[test]
    fn topics_are_promoted_to_canonical_entities() {
        let ex = ExtractedEntities {
            entities: vec![],
            topics: vec![topic("phoenix", 0.72), topic("migration", 0.60)],
            llm_importance: None,
            llm_importance_reason: None,
        };
        let out = canonicalise(&ex);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].kind, EntityKind::Topic);
        assert_eq!(out[0].canonical_id, "topic:phoenix");
        assert!((out[0].score - 0.72).abs() < 1e-6);
        assert_eq!(out[1].canonical_id, "topic:migration");
    }

    #[test]
    fn topic_canonicalisation_lowercases() {
        let ex = ExtractedEntities {
            entities: vec![],
            topics: vec![topic("Phoenix", 1.0), topic("PHOENIX", 0.5)],
            llm_importance: None,
            llm_importance_reason: None,
        };
        let out = canonicalise(&ex);
        // Both normalise to "topic:phoenix" — second occurrence is deduped.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].canonical_id, "topic:phoenix");
        // First-seen surface is preserved.
        assert_eq!(out[0].surface, "Phoenix");
    }

    #[test]
    fn hashtag_and_topic_with_same_label_coexist() {
        // "#launch" regex → EntityKind::Hashtag, LLM theme "launch" → Topic.
        // They stay as two distinct canonical entities — different kind,
        // different canonical_id prefix.
        let ex = ExtractedEntities {
            entities: vec![ExtractedEntity {
                kind: EntityKind::Hashtag,
                text: "launch".into(),
                span_start: 0,
                span_end: 6,
                score: 1.0,
            }],
            topics: vec![topic("launch", 0.8)],
            llm_importance: None,
            llm_importance_reason: None,
        };
        let out = canonicalise(&ex);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].kind, EntityKind::Hashtag);
        assert_eq!(out[0].canonical_id, "hashtag:launch");
        assert_eq!(out[1].kind, EntityKind::Topic);
        assert_eq!(out[1].canonical_id, "topic:launch");
    }

    #[test]
    fn canonicalise_mixes_entities_and_topics_in_order() {
        // Entities come first, topics appended after — downstream callers
        // (e.g. routing) can rely on this ordering if they ever need it.
        let ex = ExtractedEntities {
            entities: vec![entity(EntityKind::Email, "alice@example.com")],
            topics: vec![topic("phoenix", 0.7)],
            llm_importance: None,
            llm_importance_reason: None,
        };
        let out = canonicalise(&ex);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].kind, EntityKind::Email);
        assert_eq!(out[1].kind, EntityKind::Topic);
    }

    #[test]
    fn topic_entity_kind_round_trips_through_parse() {
        // Defence in depth: ensure the new Topic variant survives the
        // round-trip used by mem_tree_entity_index on read.
        assert_eq!(EntityKind::parse("topic"), Ok(EntityKind::Topic));
        assert_eq!(EntityKind::Topic.as_str(), "topic");
    }
}
