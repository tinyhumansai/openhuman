//! Semantic validation rules for knowledge-graph relations and `ExtractionAccumulator` impl.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::ingestion_regex::sanitize_entity_name;
use super::ingestion_types::{ExtractionAccumulator, RawEntity, RawRelation};
use crate::openhuman::memory::UnifiedMemory;

/// A validation rule for semantic relationships.
#[derive(Debug)]
pub(super) struct RelationRule {
    /// Canonical predicate name (uppercase snake_case).
    pub(super) canonical: &'static str,
    /// Allowed classifications for the subject.
    pub(super) allowed_head: &'static [&'static str],
    /// Allowed classifications for the object.
    pub(super) allowed_tail: &'static [&'static str],
}

const PERSON_TYPES: &[&str] = &["PERSON"];
const ORG_TYPES: &[&str] = &[
    "ORGANIZATION",
    "PROJECT",
    "PRODUCT",
    "TOOL",
    "TOPIC",
    "WORK_ITEM",
];
const PLACE_TYPES: &[&str] = &["PLACE", "LOCATION", "ROOM"];
const DATE_TYPES: &[&str] = &["DATE"];

/// Returns the semantic validation rule for a given predicate name.
pub(super) fn relation_rule(predicate: &str) -> Option<RelationRule> {
    let normalized = UnifiedMemory::normalize_graph_predicate(predicate);
    let rule = match normalized.as_str() {
        "OWNS" | "WORKS_ON" | "RESPONSIBLE_FOR" | "REVIEWS" => RelationRule {
            canonical: "OWNS",
            allowed_head: PERSON_TYPES,
            allowed_tail: ORG_TYPES,
        },
        "USES" | "KEEPS" | "ADOPTS" => RelationRule {
            canonical: "USES",
            allowed_head: ORG_TYPES,
            allowed_tail: ORG_TYPES,
        },
        "WORKS_FOR" => RelationRule {
            canonical: "WORKS_FOR",
            allowed_head: PERSON_TYPES,
            allowed_tail: &["ORGANIZATION", "PROJECT", "PRODUCT"],
        },
        "DEPENDS_ON" => RelationRule {
            canonical: "DEPENDS_ON",
            allowed_head: ORG_TYPES,
            allowed_tail: ORG_TYPES,
        },
        "PREFERS" => RelationRule {
            canonical: "PREFERS",
            allowed_head: PERSON_TYPES,
            allowed_tail: &["TOPIC", "WORK_ITEM", "MODE", "PRODUCT", "TOOL"],
        },
        "HAS_DEADLINE" | "DUE_ON" => RelationRule {
            canonical: "HAS_DEADLINE",
            allowed_head: ORG_TYPES,
            allowed_tail: DATE_TYPES,
        },
        "COMMUNICATES_WITH" => RelationRule {
            canonical: "COMMUNICATES_WITH",
            allowed_head: PERSON_TYPES,
            allowed_tail: PERSON_TYPES,
        },
        "INVESTIGATES" | "EVALUATES" => RelationRule {
            canonical: "INVESTIGATES",
            allowed_head: PERSON_TYPES,
            allowed_tail: ORG_TYPES,
        },
        "NORTH_OF" => RelationRule {
            canonical: "NORTH_OF",
            allowed_head: PLACE_TYPES,
            allowed_tail: PLACE_TYPES,
        },
        "SOUTH_OF" => RelationRule {
            canonical: "SOUTH_OF",
            allowed_head: PLACE_TYPES,
            allowed_tail: PLACE_TYPES,
        },
        "EAST_OF" => RelationRule {
            canonical: "EAST_OF",
            allowed_head: PLACE_TYPES,
            allowed_tail: PLACE_TYPES,
        },
        "WEST_OF" => RelationRule {
            canonical: "WEST_OF",
            allowed_head: PLACE_TYPES,
            allowed_tail: PLACE_TYPES,
        },
        "AVOIDS" => RelationRule {
            canonical: "AVOIDS",
            allowed_head: ORG_TYPES,
            allowed_tail: ORG_TYPES,
        },
        _ => return None,
    };
    Some(rule)
}

/// Helper to check if a classification is allowed by a rule.
pub(super) fn type_allowed(actual: &str, allowed: &[&str]) -> bool {
    allowed.is_empty() || allowed.iter().any(|candidate| candidate == &actual)
}

/// Resolves a person's name using the known alias map.
pub(super) fn resolve_person_alias(
    name: &str,
    known_people: &std::collections::HashMap<String, String>,
) -> String {
    let upper = name.to_uppercase();
    known_people.get(&upper).cloned().unwrap_or(upper)
}

impl ExtractionAccumulator {
    /// Ingests a full name and its components (e.g., first name) into the alias map.
    pub(super) fn remember_person_aliases(&mut self, canonical_name: &str) {
        let parts = canonical_name.split_whitespace().collect::<Vec<_>>();
        if let Some(first_name) = parts.first() {
            self.known_people
                .entry(first_name.to_uppercase())
                .or_insert_with(|| canonical_name.to_string());
        }
    }

    /// Records a new entity, updating confidence if already known.
    pub(super) fn add_entity(
        &mut self,
        name: &str,
        entity_type: &str,
        confidence: f32,
    ) -> Option<String> {
        let cleaned = sanitize_entity_name(name);
        if cleaned.is_empty() {
            return None;
        }
        let resolved_name = if entity_type == "PERSON" {
            resolve_person_alias(&cleaned, &self.known_people)
        } else {
            cleaned.clone()
        };
        let entry = self
            .entities
            .entry(resolved_name.clone())
            .or_insert_with(|| RawEntity {
                name: resolved_name.clone(),
                entity_type: entity_type.to_string(),
                confidence,
            });
        if confidence > entry.confidence {
            entry.confidence = confidence;
        }
        if entity_type == "PERSON" {
            self.remember_person_aliases(&resolved_name);
        }
        Some(resolved_name)
    }

    /// Records a new relationship, applying semantic validation rules.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn add_relation(
        &mut self,
        subject: &str,
        subject_type: &str,
        predicate: &str,
        object: &str,
        object_type: &str,
        confidence: f32,
        chunk_index: usize,
        order_index: i64,
        metadata: Map<String, Value>,
    ) {
        let Some(rule) = relation_rule(predicate) else {
            return;
        };
        let Some(subject_name) = self.add_entity(subject, subject_type, confidence) else {
            return;
        };
        let Some(object_name) = self.add_entity(object, object_type, confidence) else {
            return;
        };
        if subject_name == object_name {
            return;
        }
        let actual_subject_type = self
            .entities
            .get(&subject_name)
            .map(|value| value.entity_type.as_str())
            .unwrap_or(subject_type);
        let actual_object_type = self
            .entities
            .get(&object_name)
            .map(|value| value.entity_type.as_str())
            .unwrap_or(object_type);
        if !type_allowed(actual_subject_type, rule.allowed_head)
            || !type_allowed(actual_object_type, rule.allowed_tail)
        {
            return;
        }

        let mut chunk_indexes = BTreeSet::new();
        chunk_indexes.insert(chunk_index);
        self.relations.push(RawRelation {
            subject: subject_name,
            subject_type: actual_subject_type.to_string(),
            predicate: rule.canonical.to_string(),
            object: object_name,
            object_type: actual_object_type.to_string(),
            confidence,
            chunk_indexes,
            order_index,
            metadata,
        });
    }
}
