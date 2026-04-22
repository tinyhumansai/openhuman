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
pub fn canonicalise(extracted: &ExtractedEntities) -> Vec<CanonicalEntity> {
    extracted
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
        .collect()
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
}
