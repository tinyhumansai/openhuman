//! Query-side NLP for the deterministic (E2GraphRAG) retriever.
//!
//! [`extract_query_entities`] turns a natural-language query into a set of
//! canonical entity ids that key into `mem_tree_entity_index` and the
//! co-occurrence graph. It prefers the runtime Python server's spaCy backend
//! (named entities +
//! salient nouns) and falls back to the in-Rust regex extractor whenever
//! spaCy is disabled or unavailable — so retrieval always works offline, just
//! with lower person/org recall.
//!
//! Output is intentionally `Vec<CanonicalEntity>`: it reuses
//! [`score::resolver::canonicalise`] so query entity ids land in the exact
//! same `<kind>:<value>` namespace as the indexed chunk entities. No id
//! mismatch, no bespoke join.

pub use crate::openhuman::runtime_python_server::{
    ensure_spacy, spacy_provisioned, SpacyResponse, SPACY_MODEL,
};

use crate::openhuman::config::Config;
use crate::openhuman::memory_tree::score::extract::{
    EntityKind, ExtractedEntities, ExtractedEntity, ExtractedTopic,
};
use crate::openhuman::memory_tree::score::resolver::{canonicalise, CanonicalEntity};

/// Map a spaCy entity label to our [`EntityKind`]. Unknown labels collapse to
/// [`EntityKind::Misc`] so they still participate as graph anchors.
fn map_spacy_label(label: &str) -> EntityKind {
    match label {
        "PERSON" => EntityKind::Person,
        "ORG" | "NORP" => EntityKind::Organization,
        "GPE" | "LOC" | "FAC" => EntityKind::Location,
        "PRODUCT" => EntityKind::Product,
        "EVENT" => EntityKind::Event,
        "DATE" | "TIME" => EntityKind::Datetime,
        "MONEY" | "QUANTITY" | "PERCENT" | "CARDINAL" | "ORDINAL" => EntityKind::Quantity,
        "LANGUAGE" => EntityKind::Technology,
        _ => EntityKind::Misc,
    }
}

/// Extract canonical query entities, preferring spaCy and falling back to the
/// in-Rust regex extractor. Never fails: an unavailable sidecar degrades to
/// the fallback rather than erroring, because an empty/partial entity set just
/// routes retrieval toward the global (dense) branch.
pub async fn extract_query_entities(config: &Config, query: &str) -> Vec<CanonicalEntity> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if config.memory_tree.spacy_enabled {
        match crate::openhuman::runtime_python_server::extract_spacy(config, trimmed).await {
            Ok(resp) => {
                let extracted = spacy_to_extracted(&resp);
                let canon = canonicalise(&extracted);
                log::debug!(
                    "[memory_tree::nlp] spaCy query extraction: entities={} nouns={} canonical={}",
                    resp.entities.len(),
                    resp.nouns.len(),
                    canon.len()
                );
                return canon;
            }
            Err(e) => {
                log::warn!("[memory_tree::nlp] spaCy extraction failed, falling back: {e:#}");
            }
        }
    } else {
        log::debug!("[memory_tree::nlp] spaCy disabled by config — using regex fallback");
    }

    fallback_extract(trimmed).await
}

/// Build [`ExtractedEntities`] from a spaCy response: named entities become
/// entity spans, salient nouns become topics. Topics are promoted to
/// `topic:<noun>` canonical ids by [`canonicalise`].
fn spacy_to_extracted(resp: &SpacyResponse) -> ExtractedEntities {
    let entities = resp
        .entities
        .iter()
        .map(|e| ExtractedEntity {
            kind: map_spacy_label(&e.label),
            text: e.text.clone(),
            span_start: e.start,
            span_end: e.end,
            score: 1.0,
        })
        .collect();
    let topics = resp
        .nouns
        .iter()
        .map(|n| ExtractedTopic {
            label: n.clone(),
            score: 1.0,
        })
        .collect();
    ExtractedEntities {
        entities,
        topics,
        llm_importance: None,
        llm_importance_reason: None,
    }
}

/// Regex-only fallback. Deterministic, no network, no LLM — catches
/// emails/urls/handles/hashtags in the query. Person/org recall is lost
/// (spaCy's job), which simply biases retrieval toward the global branch.
async fn fallback_extract(query: &str) -> Vec<CanonicalEntity> {
    use crate::openhuman::memory_tree::score::extract::{CompositeExtractor, EntityExtractor};
    let extractor = CompositeExtractor::regex_only();
    match extractor.extract(query).await {
        Ok(extracted) => {
            let canon = canonicalise(&extracted);
            log::debug!(
                "[memory_tree::nlp] regex fallback query extraction: canonical={}",
                canon.len()
            );
            canon
        }
        Err(e) => {
            log::warn!("[memory_tree::nlp] regex fallback failed: {e:#}");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_spacy_off() -> Config {
        let mut c = Config::default();
        c.memory_tree.spacy_enabled = false;
        c
    }

    #[test]
    fn label_mapping_covers_common_kinds() {
        assert_eq!(map_spacy_label("PERSON"), EntityKind::Person);
        assert_eq!(map_spacy_label("ORG"), EntityKind::Organization);
        assert_eq!(map_spacy_label("GPE"), EntityKind::Location);
        assert_eq!(map_spacy_label("WHATEVER"), EntityKind::Misc);
    }

    #[tokio::test]
    async fn fallback_used_when_spacy_disabled_extracts_mechanical_entities() {
        let cfg = cfg_spacy_off();
        let ents = extract_query_entities(&cfg, "ping alice@example.com about #launch").await;
        assert!(
            ents.iter()
                .any(|e| e.canonical_id == "email:alice@example.com"),
            "regex fallback should find the email; got {ents:?}"
        );
        assert!(
            ents.iter().any(|e| e.kind == EntityKind::Hashtag),
            "regex fallback should find the hashtag; got {ents:?}"
        );
    }

    #[tokio::test]
    async fn empty_query_yields_no_entities() {
        let cfg = cfg_spacy_off();
        assert!(extract_query_entities(&cfg, "   ").await.is_empty());
    }

    #[test]
    fn spacy_response_maps_nouns_to_topics() {
        let resp = SpacyResponse {
            entities: vec![
                crate::openhuman::runtime_python_server::spacy::SpacyEntity {
                    text: "Alice".into(),
                    label: "PERSON".into(),
                    start: 0,
                    end: 5,
                },
            ],
            nouns: vec!["migration".into()],
        };
        let extracted = spacy_to_extracted(&resp);
        let canon = canonicalise(&extracted);
        assert!(canon.iter().any(|c| c.canonical_id == "person:alice"));
        assert!(canon.iter().any(|c| c.canonical_id == "topic:migration"));
    }
}
