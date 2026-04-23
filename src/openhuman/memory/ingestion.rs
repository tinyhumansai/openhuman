//! Document ingestion and knowledge extraction for the OpenHuman memory system.
//!
//! This module provides the pipeline for taking raw unstructured text and
//! transforming it into structured memory. The process includes:
//! 1. **Chunking**: Splitting the document into manageable pieces.
//! 2. **Structured Extraction**: Using regex-based rules to identify known patterns
//!    (e.g., email headers, specific project labels).
//! 3. **Heuristic Extraction**: Using rule-based parsing to identify entities
//!    and their relationships.
//! 4. **Aggregation**: Resolving aliases, merging duplicates, and normalizing names.
//! 5. **Persistence**: Upserting the document, text chunks, and graph relations into
//!    the memory store.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::openhuman::memory::store::types::NamespaceDocumentInput;
use crate::openhuman::memory::UnifiedMemory;

/// Default extraction backend label reported in ingestion metadata.
pub const DEFAULT_MEMORY_EXTRACTION_MODEL: &str = "heuristic-only";
/// Default number of tokens per text chunk during ingestion.
const DEFAULT_CHUNK_TOKENS: usize = 225;

/// Granularity of extraction for heuristic parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ExtractionMode {
    /// Extract from each individual sentence (higher precision).
    #[default]
    Sentence,
    /// Extract from the entire chunk at once (faster, better for context).
    Chunk,
}

impl ExtractionMode {
    /// Returns the string representation of the extraction mode.
    fn as_str(self) -> &'static str {
        match self {
            Self::Sentence => "sentence",
            Self::Chunk => "chunk",
        }
    }
}

/// Configuration for the memory ingestion process.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryIngestionConfig {
    /// Extraction backend label recorded in metadata/results.
    pub model_name: String,
    /// The granularity of heuristic extraction.
    #[serde(default)]
    pub extraction_mode: ExtractionMode,
    /// Minimum confidence threshold for entity extraction (0.0 to 1.0).
    #[serde(default = "default_entity_threshold")]
    pub entity_threshold: f32,
    /// Minimum confidence threshold for relation extraction (0.0 to 1.0).
    #[serde(default = "default_relation_threshold")]
    pub relation_threshold: f32,
    /// Threshold for adjacency-based heuristics.
    #[serde(default = "default_adjacency_threshold")]
    pub adjacency_threshold: f32,
    /// Reserved batch-size knob kept for config compatibility.
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
}

fn default_entity_threshold() -> f32 {
    0.45
}

fn default_relation_threshold() -> f32 {
    0.30
}

fn default_adjacency_threshold() -> f32 {
    0.50
}

fn default_batch_size() -> usize {
    16
}

impl Default for MemoryIngestionConfig {
    fn default() -> Self {
        Self {
            model_name: DEFAULT_MEMORY_EXTRACTION_MODEL.to_string(),
            extraction_mode: ExtractionMode::Sentence,
            entity_threshold: default_entity_threshold(),
            relation_threshold: default_relation_threshold(),
            adjacency_threshold: default_adjacency_threshold(),
            batch_size: default_batch_size(),
        }
    }
}

/// A request to ingest a single document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryIngestionRequest {
    /// The document input to process.
    pub document: NamespaceDocumentInput,
    /// Ingestion configuration.
    #[serde(default)]
    pub config: MemoryIngestionConfig,
}

/// An entity identified during the ingestion process.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedEntity {
    /// Normalized name of the entity (all-caps).
    pub name: String,
    /// Classification (e.g., PERSON, ORGANIZATION).
    pub entity_type: String,
    /// Known aliases for this entity.
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// A relation identified during the ingestion process.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedRelation {
    /// Name of the subject entity.
    pub subject: String,
    /// Classification of the subject.
    pub subject_type: String,
    /// Relationship type (e.g., OWNS, WORKS_ON).
    pub predicate: String,
    /// Name of the object entity.
    pub object: String,
    /// Classification of the object.
    pub object_type: String,
    /// Extraction confidence (0.0 to 1.0).
    pub confidence: f32,
    /// Number of distinct occurrences of this relation.
    pub evidence_count: u32,
    /// IDs of the chunks where this relation was found.
    pub chunk_ids: Vec<String>,
    /// Sequential order index for reconstruction.
    pub order_index: Option<i64>,
    /// Additional metadata about the extraction.
    pub metadata: Value,
}

/// The comprehensive result of an ingestion operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryIngestionResult {
    /// ID of the document that was ingested.
    pub document_id: String,
    /// Namespace containing the document.
    pub namespace: String,
    /// Extraction backend label recorded for the ingestion run.
    pub model_name: String,
    /// Mode used for extraction.
    pub extraction_mode: String,
    /// Total number of chunks processed.
    pub chunk_count: usize,
    /// Total number of distinct entities found.
    pub entity_count: usize,
    /// Total number of distinct relations found.
    pub relation_count: usize,
    /// Number of identified user preferences.
    pub preference_count: usize,
    /// Number of identified decisions.
    pub decision_count: usize,
    /// Auto-generated tags for the document.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Complete list of identified entities.
    #[serde(default)]
    pub entities: Vec<ExtractedEntity>,
    /// Complete list of identified relations.
    #[serde(default)]
    pub relations: Vec<ExtractedRelation>,
}

/// Intermediate representation of an entity before normalization and alias resolution.
#[derive(Debug, Clone)]
struct RawEntity {
    name: String,
    entity_type: String,
    confidence: f32,
}

/// Intermediate representation of a relationship before aggregation.
#[derive(Debug, Clone)]
struct RawRelation {
    subject: String,
    subject_type: String,
    predicate: String,
    object: String,
    object_type: String,
    confidence: f32,
    /// Indices of the chunks where this relation was found.
    chunk_indexes: BTreeSet<usize>,
    /// Global sequential index for ordering within the document.
    order_index: i64,
    /// JSON metadata for the relation.
    metadata: Map<String, Value>,
}

/// A single unit of text (sentence or chunk) passed to the extractor.
#[derive(Debug, Clone)]
struct ExtractionUnit {
    text: String,
    chunk_index: usize,
    order_index: i64,
}

/// Accumulates extraction results across multiple chunks or units.
///
/// Handles entity and relation deduplication, alias tracking, and
/// basic document understanding (e.g., identifying the primary subject).
#[derive(Debug, Default)]
struct ExtractionAccumulator {
    /// Mapping of normalized entity name to its highest-confidence raw extraction.
    entities: HashMap<String, RawEntity>,
    /// Collected relations before final canonicalization.
    relations: Vec<RawRelation>,
    /// Tags identified during processing.
    tags: BTreeSet<String>,
    /// Decisions identified during processing.
    decisions: BTreeSet<String>,
    /// User preferences identified during processing.
    preferences: BTreeSet<String>,
    /// Inferred document kind (e.g., "profile").
    doc_kind: Option<String>,
    /// The document's inferred primary subject.
    primary_subject: Option<String>,
    /// Sanitized document title.
    document_title: Option<String>,
    /// The subject of the current markdown section.
    current_subject: Option<String>,
    /// Current sender if processing a message/thread.
    current_sender: Option<String>,
    /// Mapping of names to their canonicalized full name.
    known_people: HashMap<String, String>,
}

/// The result of the parsing stage of ingestion.
#[derive(Debug)]
struct ParsedIngestion {
    tags: Vec<String>,
    metadata: Value,
    entities: Vec<ExtractedEntity>,
    relations: Vec<ExtractedRelation>,
    chunk_count: usize,
    preference_count: usize,
    decision_count: usize,
}

/// A validation rule for semantic relationships.
#[derive(Debug)]
struct RelationRule {
    /// Canonical predicate name (uppercase snake_case).
    canonical: &'static str,
    /// Allowed classifications for the subject.
    allowed_head: &'static [&'static str],
    /// Allowed classifications for the object.
    allowed_tail: &'static [&'static str],
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
fn relation_rule(predicate: &str) -> Option<RelationRule> {
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
fn type_allowed(actual: &str, allowed: &[&str]) -> bool {
    allowed.is_empty() || allowed.iter().any(|candidate| candidate == &actual)
}

/// Resolves a person's name using the known alias map.
fn resolve_person_alias(name: &str, known_people: &HashMap<String, String>) -> String {
    let upper = name.to_uppercase();
    known_people.get(&upper).cloned().unwrap_or(upper)
}

impl ExtractionAccumulator {
    /// Ingests a full name and its components (e.g., first name) into the alias map.
    fn remember_person_aliases(&mut self, canonical_name: &str) {
        let parts = canonical_name.split_whitespace().collect::<Vec<_>>();
        if let Some(first_name) = parts.first() {
            self.known_people
                .entry(first_name.to_uppercase())
                .or_insert_with(|| canonical_name.to_string());
        }
    }

    /// Records a new entity, updating confidence if already known.
    fn add_entity(&mut self, name: &str, entity_type: &str, confidence: f32) -> Option<String> {
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
    fn add_relation(
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

/// Regex for identifying standard email headers (From, To, Cc).
fn email_header_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r"^(From|To|Cc):\s*(?P<value>.+)$").expect("email header regex"))
}

/// Regex for identifying named email addresses (e.g., "John Doe <john@example.com>").
fn named_email_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?P<name>[^,<]+?)\s*<(?P<email>[^>]+)>").expect("named email regex")
    })
}

/// Regex for identifying explicit graph facts (e.g., "Alice works_on Project-X").
fn graph_fact_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"^(?P<subject>[A-Za-z0-9][A-Za-z0-9 ._\-/]+?)\s+(?P<predicate>works_on|depends_on|uses|evaluates|owns|prefers)\s+(?P<object>.+)$",
        )
        .expect("graph fact regex")
    })
}

/// Regex for identifying ownership patterns (e.g., "Bob owns the repository").
fn explicit_owner_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^(?P<subject>[A-Za-z][A-Za-z ._-]+?) owns (?P<object>.+)$")
            .expect("explicit owner regex")
    })
}

/// Regex for identifying preference patterns (e.g., "Carol prefers light mode").
fn explicit_preference_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^(?P<subject>[A-Za-z][A-Za-z ._-]+?) prefers (?P<object>.+)$")
            .expect("explicit preference regex")
    })
}

/// Regex for identifying action items or assignments (e.g., "Dave: finish the API").
fn action_item_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^(?P<subject>[A-Za-z][A-Za-z ._-]+?):\s*(?P<object>.+)$")
            .expect("action item regex")
    })
}

/// Regex for identifying review assignments.
fn will_review_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^(?P<subject>[A-Za-z][A-Za-z ._-]+?) will review (?P<object>.+)$")
            .expect("will review regex")
    })
}

/// Regex for identifying complex giving/receiving interactions.
fn recipient_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)(?P<giver>[A-Z][A-Za-z]+(?: [A-Z][A-Za-z]+)?)\s+(gave|sent|handed|passed)\s+(?P<object>.+?)\s+to\s+(?P<recipient>[A-Z][A-Za-z]+(?: [A-Z][A-Za-z]+)?)",
        )
        .expect("recipient regex")
    })
}

/// Regex for identifying spatial relationships (e.g., "Kitchen is north of the Garden").
fn spatial_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)(?P<head>[A-Za-z][A-Za-z0-9 _-]+?)\s+is\s+(?P<direction>north|south|east|west)\s+of\s+(?P<tail>[A-Za-z][A-Za-z0-9 _-]+)",
        )
        .expect("spatial regex")
    })
}

/// Regex for identifying dates in "Month DD, YYYY" format.
fn month_date_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)\b(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Sept|Oct|Nov|Dec)[a-z]*\s+\d{1,2},\s+\d{4}\b")
            .expect("month date regex")
    })
}

/// Regex for identifying ISO-8601 dates (YYYY-MM-DD).
fn iso_date_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\b\d{4}-\d{2}-\d{2}\b").expect("iso date regex"))
}

/// Regex for identifying potential person names (Title Case).
fn person_name_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r"\b[A-Z][a-z]+(?: [A-Z][a-z]+)+\b").expect("person name regex"))
}

/// Normalizes an entity name by trimming punctuation, collapsing whitespace, and converting to uppercase.
fn sanitize_entity_name(name: &str) -> String {
    let trimmed = name.trim().trim_matches(|ch: char| {
        matches!(ch, '-' | ':' | ';' | ',' | '.' | '"' | '\'' | '(' | ')')
    });
    if trimmed.is_empty() {
        return String::new();
    }
    UnifiedMemory::collapse_whitespace(trimmed).to_uppercase()
}

/// Normalizes text content by trimming and collapsing whitespace.
fn sanitize_fact_text(text: &str) -> String {
    let trimmed = text
        .trim()
        .trim_start_matches('-')
        .trim()
        .trim_matches(|ch: char| matches!(ch, ':' | ';' | ',' | '.'));
    UnifiedMemory::collapse_whitespace(trimmed)
}

/// Heuristically classifies an entity based on its name and known person map.
fn classify_entity(name: &str, known_people: &HashMap<String, String>) -> &'static str {
    let upper = sanitize_entity_name(name);
    if upper.is_empty() {
        return "TOPIC";
    }
    if month_date_regex().is_match(name) || iso_date_regex().is_match(name) {
        return "DATE";
    }
    if upper.contains('@') {
        return "ORGANIZATION";
    }
    if known_people.contains_key(&upper) || person_name_regex().is_match(name) {
        return "PERSON";
    }
    if matches!(
        upper.as_str(),
        "OPENHUMAN" | "JSON-RPC" | "JSON-RPC 2.0" | "NEOCORTEX_V2" | "NEOCORTEX V2"
    ) {
        return "PRODUCT";
    }
    if upper.contains("MODEL") {
        return "TOOL";
    }
    if upper.contains("MODE") {
        return "MODE";
    }
    if upper.contains("MILESTONE")
        || upper.contains("ROADMAP")
        || upper.contains("CONTRACT")
        || upper.contains("API")
        || upper.contains("MEMORY")
        || upper.contains("FIXTURE")
        || upper.contains("THREAD")
        || upper.contains("WORK")
    {
        return "WORK_ITEM";
    }
    if upper.contains("OFFICE")
        || upper.contains("ROOM")
        || upper.contains("GARDEN")
        || upper.contains("KITCHEN")
    {
        return "ROOM";
    }
    if upper.contains("TINYHUMANS") || upper.ends_with("CORE") {
        return "ORGANIZATION";
    }
    if (upper.contains('-') || upper.contains('_')) && !upper.contains(' ') {
        return "PROJECT";
    }
    "TOPIC"
}

/// Splits a document into individual sentences based on punctuation and line breaks.
fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if matches!(ch, '.' | '!' | '?' | '\n') {
            let candidate = sanitize_fact_text(&current);
            if !candidate.is_empty() {
                out.push(candidate);
            }
            current.clear();
        }
    }
    let tail = sanitize_fact_text(&current);
    if !tail.is_empty() {
        out.push(tail);
    }
    let mut merged: Vec<String> = Vec::new();
    for sentence in out {
        if sentence.len() < 5 && !merged.is_empty() {
            if let Some(last) = merged.last_mut() {
                last.push(' ');
                last.push_str(&sentence);
            }
        } else {
            merged.push(sentence);
        }
    }
    if merged.is_empty() && !text.trim().is_empty() {
        merged.push(sanitize_fact_text(text));
    }
    merged
}

/// Groups chunks into extraction units based on the configured mode.
fn build_units(chunks: &[String], mode: ExtractionMode) -> Vec<ExtractionUnit> {
    let mut units = Vec::new();
    let mut order_index = 0_i64;
    for (chunk_index, chunk) in chunks.iter().enumerate() {
        match mode {
            ExtractionMode::Chunk => {
                let text = sanitize_fact_text(chunk);
                if text.is_empty() {
                    continue;
                }
                units.push(ExtractionUnit {
                    text,
                    chunk_index,
                    order_index,
                });
                order_index += 1;
            }
            ExtractionMode::Sentence => {
                for sentence in split_sentences(chunk) {
                    if sentence.is_empty() {
                        continue;
                    }
                    units.push(ExtractionUnit {
                        text: sentence,
                        chunk_index,
                        order_index,
                    });
                    order_index += 1;
                }
            }
        }
    }
    units
}

/// Searches for the chunk index that most likely contains the given excerpt.
fn find_chunk_index(chunks: &[String], excerpt: &str, hint: usize) -> usize {
    if chunks.is_empty() {
        return 0;
    }
    let needle = UnifiedMemory::normalize_search_text(excerpt);
    if needle.is_empty() {
        return hint.min(chunks.len().saturating_sub(1));
    }
    for (index, chunk) in chunks.iter().enumerate().skip(hint) {
        if UnifiedMemory::normalize_search_text(chunk).contains(&needle) {
            return index;
        }
    }
    for (index, chunk) in chunks.iter().enumerate().take(hint.min(chunks.len())) {
        if UnifiedMemory::normalize_search_text(chunk).contains(&needle) {
            return index;
        }
    }
    hint.min(chunks.len().saturating_sub(1))
}

fn extract_people_from_header(value: &str, accumulator: &mut ExtractionAccumulator) -> Vec<String> {
    let mut people = Vec::new();
    for captures in named_email_regex().captures_iter(value) {
        let name = sanitize_fact_text(
            captures
                .name("name")
                .map(|value| value.as_str())
                .unwrap_or(""),
        );
        if name.is_empty() {
            continue;
        }
        let canonical = sanitize_entity_name(&name);
        let _ = accumulator.add_entity(&canonical, "PERSON", 0.95);
        accumulator.remember_person_aliases(&canonical);
        people.push(canonical);
    }
    people
}

fn detect_primary_subject(text: &str) -> Option<String> {
    if text.contains("OpenHuman") {
        return Some("OPENHUMAN".to_string());
    }
    None
}

fn enrich_document_metadata(
    input: &NamespaceDocumentInput,
    parsed: &ParsedIngestion,
    config: &MemoryIngestionConfig,
) -> (NamespaceDocumentInput, Vec<String>) {
    let mut metadata = match input.metadata.clone() {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    for (key, value) in parsed.metadata.as_object().cloned().unwrap_or_default() {
        metadata.insert(key, value);
    }
    metadata.insert(
        "ingestion".to_string(),
        json!({
            "backend": "openhuman_rust_heuristic",
            "model_name": config.model_name,
            "extraction_mode": config.extraction_mode.as_str(),
            "entity_count": parsed.entities.len(),
            "relation_count": parsed.relations.len(),
            "preference_count": parsed.preference_count,
            "decision_count": parsed.decision_count,
            "chunk_count": parsed.chunk_count,
        }),
    );
    if parsed.preference_count > 0 || parsed.decision_count > 0 {
        metadata.insert("kind".to_string(), json!("profile"));
    }

    let mut tags = input.tags.iter().cloned().collect::<BTreeSet<_>>();
    tags.extend(parsed.tags.iter().cloned());
    let tags = tags.into_iter().collect::<Vec<_>>();

    (
        NamespaceDocumentInput {
            namespace: input.namespace.clone(),
            key: input.key.clone(),
            title: input.title.clone(),
            content: input.content.clone(),
            source_type: input.source_type.clone(),
            priority: input.priority.clone(),
            tags: tags.clone(),
            metadata: Value::Object(metadata),
            category: input.category.clone(),
            session_id: input.session_id.clone(),
            document_id: input.document_id.clone(),
        },
        tags,
    )
}

fn reverse_aliases(aliases: &HashMap<String, String>) -> BTreeMap<String, Vec<String>> {
    let mut reverse = BTreeMap::new();
    for (alias, canonical) in aliases {
        if alias == canonical {
            continue;
        }
        reverse
            .entry(canonical.clone())
            .or_insert_with(Vec::new)
            .push(alias.clone());
    }
    for values in reverse.values_mut() {
        values.sort();
        values.dedup();
    }
    reverse
}

fn build_alias_map(entities: &HashMap<String, RawEntity>) -> HashMap<String, String> {
    let mut by_type = HashMap::<String, Vec<String>>::new();
    for entity in entities.values() {
        by_type
            .entry(entity.entity_type.clone())
            .or_default()
            .push(entity.name.clone());
    }

    let mut aliases = HashMap::new();
    for names in by_type.values_mut() {
        names.sort_by_key(|name| std::cmp::Reverse(name.len()));
        for short in names.iter() {
            for long in names.iter() {
                if short == long || long.len() <= short.len() {
                    continue;
                }
                if long.starts_with(&format!("{short} ")) || long.ends_with(&format!(" {short}")) {
                    aliases.entry(short.clone()).or_insert_with(|| long.clone());
                    break;
                }
            }
        }
    }
    aliases
}

fn resolve_alias(name: &str, aliases: &HashMap<String, String>) -> String {
    let mut current = name.to_string();
    let mut seen = BTreeSet::new();
    while let Some(next) = aliases.get(&current) {
        if !seen.insert(current.clone()) {
            break;
        }
        current = next.clone();
    }
    current
}

async fn parse_document(
    content: &str,
    title: &str,
    config: &MemoryIngestionConfig,
) -> ParsedIngestion {
    let chunks = UnifiedMemory::chunk_document_content(content, DEFAULT_CHUNK_TOKENS);
    log::info!(
        "[memory:ingestion] parse_document title={title:?} model={} \
         content_len={} chunk_count={} — heuristic extraction active",
        config.model_name,
        content.len(),
        chunks.len(),
    );
    let mut accumulator = ExtractionAccumulator {
        document_title: Some(sanitize_entity_name(title)),
        primary_subject: detect_primary_subject(title),
        ..ExtractionAccumulator::default()
    };

    let mut chunk_hint = 0_usize;
    for raw_line in content.lines() {
        let line = sanitize_fact_text(raw_line);
        if line.is_empty() {
            continue;
        }

        let chunk_index = find_chunk_index(&chunks, &line, chunk_hint);
        chunk_hint = chunk_index;
        let order_index = i64::try_from(chunk_index).unwrap_or(i64::MAX);

        if raw_line.trim_start().starts_with('#') {
            let heading = sanitize_entity_name(raw_line.trim_start_matches('#'));
            if !heading.is_empty() {
                if accumulator.document_title.is_none() {
                    accumulator.document_title = Some(heading.clone());
                }
                accumulator.current_subject = Some(heading);
            }
            continue;
        }

        if let Some(captures) = email_header_regex().captures(&line) {
            let header_name = captures
                .get(1)
                .map(|value| value.as_str())
                .unwrap_or_default()
                .to_ascii_uppercase();
            let value = captures
                .name("value")
                .map(|value| value.as_str())
                .unwrap_or("");
            let people = extract_people_from_header(value, &mut accumulator);
            if header_name == "FROM" {
                accumulator.current_sender = people.first().cloned();
            } else if header_name == "TO" || header_name == "CC" {
                if let Some(sender) = accumulator.current_sender.clone() {
                    for recipient in &people {
                        accumulator.add_relation(
                            &sender,
                            "PERSON",
                            "communicates_with",
                            recipient,
                            "PERSON",
                            0.82,
                            chunk_index,
                            order_index,
                            Map::new(),
                        );
                    }
                }
            }
            continue;
        }

        if let Some(subject) = line.strip_prefix("Subject:") {
            let subject_text = sanitize_fact_text(subject);
            if let Some(primary_subject) = detect_primary_subject(&subject_text) {
                accumulator.primary_subject = Some(primary_subject);
            }
            continue;
        }

        if let Some(date_text) = line.strip_prefix("Date:") {
            let date_text = sanitize_fact_text(date_text);
            if let Some(sender) = accumulator.current_sender.clone() {
                accumulator.add_relation(
                    &sender,
                    "PERSON",
                    "has_deadline",
                    &date_text,
                    "DATE",
                    0.75,
                    chunk_index,
                    order_index,
                    Map::new(),
                );
            }
            continue;
        }

        if let Some(value) = line.strip_prefix("Project name:") {
            let project = sanitize_entity_name(value);
            if !project.is_empty() {
                accumulator.primary_subject = Some(project.clone());
                let _ = accumulator.add_entity(&project, "PROJECT", 0.96);
            }
            continue;
        }

        if let Some(value) = line.strip_prefix("Subproject:") {
            let subproject = sanitize_entity_name(value);
            if !subproject.is_empty() {
                let _ = accumulator.add_entity(&subproject, "PROJECT", 0.92);
            }
            continue;
        }

        if let Some(value) = line.strip_prefix("Owner:") {
            let owner = sanitize_entity_name(value);
            let owned = accumulator
                .current_subject
                .clone()
                .or_else(|| accumulator.primary_subject.clone())
                .or_else(|| accumulator.document_title.clone())
                .unwrap_or_else(|| "DOCUMENT".to_string());
            accumulator.add_relation(
                &owner,
                "PERSON",
                "owns",
                &owned,
                "WORK_ITEM",
                0.94,
                chunk_index,
                order_index,
                Map::new(),
            );
            continue;
        }

        if let Some(value) = line.strip_prefix("Name:") {
            let name = sanitize_entity_name(value);
            if !name.is_empty() {
                accumulator.current_subject = Some(name.clone());
                let _ = accumulator.add_entity(&name, "WORK_ITEM", 0.93);
            }
            continue;
        }

        if let Some(value) = line.strip_prefix("Due date:") {
            let due_date = sanitize_fact_text(value);
            let subject = accumulator
                .current_subject
                .clone()
                .or_else(|| accumulator.primary_subject.clone())
                .unwrap_or_else(|| "DOCUMENT".to_string());
            accumulator.add_relation(
                &subject,
                "WORK_ITEM",
                "has_deadline",
                &due_date,
                "DATE",
                0.92,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator.tags.insert("deadline".to_string());
            continue;
        }

        if let Some(value) = line.strip_prefix("Target milestone:") {
            let due_date = sanitize_fact_text(value);
            let subject = accumulator
                .primary_subject
                .clone()
                .or_else(|| accumulator.document_title.clone())
                .unwrap_or_else(|| "DOCUMENT".to_string());
            accumulator.add_relation(
                &subject,
                "PROJECT",
                "has_deadline",
                &due_date,
                "DATE",
                0.92,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator.tags.insert("deadline".to_string());
            continue;
        }

        if let Some(value) = line.strip_prefix("Preferred embedding model for local experiments:") {
            let model = sanitize_fact_text(value);
            let subject = accumulator
                .primary_subject
                .clone()
                .or_else(|| accumulator.document_title.clone())
                .unwrap_or_else(|| "DOCUMENT".to_string());
            accumulator.add_relation(
                &subject,
                "PROJECT",
                "uses",
                &model,
                "TOOL",
                0.88,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator
                .decisions
                .insert(format!("{subject} uses {model}"));
            accumulator.tags.insert("decision".to_string());
            continue;
        }

        if let Some(value) = line.strip_prefix("Preferred extraction mode to try first:") {
            let mode = sanitize_fact_text(value);
            let subject = accumulator
                .primary_subject
                .clone()
                .or_else(|| accumulator.document_title.clone())
                .unwrap_or_else(|| "DOCUMENT".to_string());
            accumulator.add_relation(
                &subject,
                "PROJECT",
                "uses",
                &mode,
                "MODE",
                0.88,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator
                .decisions
                .insert(format!("{subject} uses {mode}"));
            accumulator.tags.insert("decision".to_string());
            continue;
        }

        if let Some(captures) = graph_fact_regex().captures(&line) {
            let subject = captures
                .name("subject")
                .map(|value| value.as_str())
                .unwrap_or("");
            let predicate = captures
                .name("predicate")
                .map(|value| value.as_str())
                .unwrap_or("");
            let object = captures
                .name("object")
                .map(|value| value.as_str())
                .unwrap_or("");
            let subject_type = classify_entity(subject, &accumulator.known_people);
            let object_type = classify_entity(object, &accumulator.known_people);
            accumulator.add_relation(
                subject,
                subject_type,
                predicate,
                object,
                object_type,
                0.87,
                chunk_index,
                order_index,
                Map::new(),
            );
            if UnifiedMemory::normalize_graph_predicate(predicate) == "PREFERS" {
                accumulator.preferences.insert(format!(
                    "{} prefers {}",
                    sanitize_entity_name(subject),
                    sanitize_fact_text(object)
                ));
                accumulator.tags.insert("preference".to_string());
                accumulator.doc_kind = Some("profile".to_string());
            }
            continue;
        }

        if let Some(captures) = explicit_owner_regex().captures(&line) {
            let subject = captures
                .name("subject")
                .map(|value| value.as_str())
                .unwrap_or("");
            let object = captures
                .name("object")
                .map(|value| value.as_str())
                .unwrap_or("");
            accumulator.add_relation(
                subject,
                "PERSON",
                "owns",
                object,
                classify_entity(object, &accumulator.known_people),
                0.94,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator.tags.insert("owner".to_string());
            continue;
        }

        if let Some(captures) = will_review_regex().captures(&line) {
            let subject = captures
                .name("subject")
                .map(|value| value.as_str())
                .unwrap_or("");
            let object = captures
                .name("object")
                .map(|value| value.as_str())
                .unwrap_or("");
            accumulator.add_relation(
                subject,
                "PERSON",
                "reviews",
                object,
                classify_entity(object, &accumulator.known_people),
                0.80,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator.tags.insert("owner".to_string());
            continue;
        }

        if let Some(captures) = explicit_preference_regex().captures(&line) {
            let subject = captures
                .name("subject")
                .map(|value| value.as_str())
                .unwrap_or("");
            let object = captures
                .name("object")
                .map(|value| value.as_str())
                .unwrap_or("");
            accumulator.add_relation(
                subject,
                "PERSON",
                "prefers",
                object,
                classify_entity(object, &accumulator.known_people),
                0.90,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator.preferences.insert(format!(
                "{} prefers {}",
                sanitize_entity_name(subject),
                sanitize_fact_text(object)
            ));
            accumulator.tags.insert("preference".to_string());
            accumulator.doc_kind = Some("profile".to_string());
            continue;
        }

        if let Some(value) = line.strip_prefix("I prefer ") {
            if let Some(subject) = accumulator.current_sender.clone() {
                let preference = sanitize_fact_text(value);
                accumulator.add_relation(
                    &subject,
                    "PERSON",
                    "prefers",
                    &preference,
                    classify_entity(&preference, &accumulator.known_people),
                    0.92,
                    chunk_index,
                    order_index,
                    Map::new(),
                );
                accumulator
                    .preferences
                    .insert(format!("{subject} prefers {preference}"));
                accumulator.tags.insert("preference".to_string());
                accumulator.doc_kind = Some("profile".to_string());
                continue;
            }
        }

        if let Some(captures) = action_item_regex().captures(&line) {
            let subject = captures
                .name("subject")
                .map(|value| value.as_str())
                .unwrap_or("");
            let object = captures
                .name("object")
                .map(|value| value.as_str())
                .unwrap_or("");
            if accumulator
                .known_people
                .contains_key(&sanitize_entity_name(subject))
                || classify_entity(subject, &accumulator.known_people) == "PERSON"
            {
                accumulator.add_relation(
                    subject,
                    "PERSON",
                    "owns",
                    object,
                    classify_entity(object, &accumulator.known_people),
                    0.83,
                    chunk_index,
                    order_index,
                    Map::new(),
                );
                accumulator.tags.insert("owner".to_string());
                continue;
            }
        }

        let upper = sanitize_entity_name(&line);
        let decision_subject = accumulator
            .primary_subject
            .clone()
            .or_else(|| accumulator.document_title.clone())
            .unwrap_or_else(|| "DOCUMENT".to_string());
        if upper.contains("JSON-RPC") {
            accumulator.add_relation(
                &decision_subject,
                "PROJECT",
                "uses",
                "JSON-RPC",
                "PRODUCT",
                0.86,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator
                .decisions
                .insert(format!("{decision_subject} uses JSON-RPC"));
            accumulator.tags.insert("decision".to_string());
            continue;
        }
        if upper.contains("SHOULD USE NAMESPACE")
            || upper.contains("USE NAMESPACE AS THE STORAGE")
            || upper.contains("NAMESPACE AS THE MAIN SCOPE KEY")
        {
            accumulator.add_relation(
                &decision_subject,
                "PROJECT",
                "uses",
                "namespace",
                "TOPIC",
                0.84,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator
                .decisions
                .insert(format!("{decision_subject} uses namespace"));
            accumulator.tags.insert("decision".to_string());
            continue;
        }
        if upper.contains("USER_ID") && (upper.contains("DO NOT NEED") || upper.contains("AVOID")) {
            accumulator.add_relation(
                &decision_subject,
                "PROJECT",
                "avoids",
                "user_id",
                "TOPIC",
                0.82,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator
                .decisions
                .insert(format!("{decision_subject} avoids user_id"));
            accumulator.tags.insert("decision".to_string());
        }
    }

    for unit in build_units(&chunks, config.extraction_mode) {
        if let Some(captures) = recipient_regex().captures(&unit.text) {
            let giver = captures
                .name("giver")
                .map(|value| value.as_str())
                .unwrap_or("");
            let object = captures
                .name("object")
                .map(|value| value.as_str())
                .unwrap_or("");
            let recipient = captures
                .name("recipient")
                .map(|value| value.as_str())
                .unwrap_or("");
            accumulator.add_relation(
                giver,
                "PERSON",
                "uses",
                object,
                classify_entity(object, &accumulator.known_people),
                config.adjacency_threshold.max(0.62),
                unit.chunk_index,
                unit.order_index,
                Map::new(),
            );
            accumulator.add_relation(
                recipient,
                "PERSON",
                "uses",
                object,
                classify_entity(object, &accumulator.known_people),
                (config.adjacency_threshold * 0.9).max(0.55),
                unit.chunk_index,
                unit.order_index,
                Map::new(),
            );
        }

        if let Some(captures) = spatial_regex().captures(&unit.text) {
            let head = captures
                .name("head")
                .map(|value| value.as_str())
                .unwrap_or("");
            let direction = captures
                .name("direction")
                .map(|value| value.as_str())
                .unwrap_or("");
            let tail = captures
                .name("tail")
                .map(|value| value.as_str())
                .unwrap_or("");
            let inverse = match direction.to_ascii_lowercase().as_str() {
                "north" => "south_of",
                "south" => "north_of",
                "east" => "west_of",
                "west" => "east_of",
                _ => "",
            };
            let predicate = format!("{direction}_of");
            accumulator.add_relation(
                head,
                "ROOM",
                &predicate,
                tail,
                "ROOM",
                config.adjacency_threshold.max(0.70),
                unit.chunk_index,
                unit.order_index,
                Map::new(),
            );
            if !inverse.is_empty() {
                accumulator.add_relation(
                    tail,
                    "ROOM",
                    inverse,
                    head,
                    "ROOM",
                    config.adjacency_threshold.max(0.70),
                    unit.chunk_index,
                    unit.order_index,
                    Map::new(),
                );
            }
        }
    }

    let aliases = build_alias_map(&accumulator.entities);
    let reverse_alias = reverse_aliases(&aliases);
    let mut canonical_entities = BTreeMap::<String, RawEntity>::new();
    for entity in accumulator.entities.values() {
        let canonical = resolve_alias(&entity.name, &aliases);
        let entry = canonical_entities
            .entry(canonical.clone())
            .or_insert_with(|| RawEntity {
                name: canonical.clone(),
                entity_type: entity.entity_type.clone(),
                confidence: entity.confidence,
            });
        if entity.confidence > entry.confidence {
            entry.confidence = entity.confidence;
            entry.entity_type = entity.entity_type.clone();
        }
    }

    let mut aggregated_relations = BTreeMap::<(String, String, String), RawRelation>::new();
    for relation in accumulator.relations {
        let subject = resolve_alias(&relation.subject, &aliases);
        let object = resolve_alias(&relation.object, &aliases);
        if subject == object {
            continue;
        }
        let key = (subject.clone(), relation.predicate.clone(), object.clone());
        let entry = aggregated_relations
            .entry(key)
            .or_insert_with(|| RawRelation {
                subject,
                subject_type: relation.subject_type.clone(),
                predicate: relation.predicate.clone(),
                object,
                object_type: relation.object_type.clone(),
                confidence: relation.confidence,
                chunk_indexes: relation.chunk_indexes.clone(),
                order_index: relation.order_index,
                metadata: relation.metadata.clone(),
            });
        entry.confidence = entry.confidence.max(relation.confidence);
        entry.order_index = entry.order_index.min(relation.order_index);
        entry.chunk_indexes.extend(relation.chunk_indexes);
    }

    let entities = canonical_entities
        .into_values()
        .filter(|entity| entity.confidence >= config.entity_threshold)
        .map(|entity| ExtractedEntity {
            name: entity.name.clone(),
            entity_type: entity.entity_type,
            aliases: reverse_alias.get(&entity.name).cloned().unwrap_or_default(),
        })
        .collect::<Vec<_>>();

    let relations = aggregated_relations
        .into_values()
        .filter(|relation| relation.confidence >= config.relation_threshold)
        .map(|relation| ExtractedRelation {
            subject: relation.subject,
            subject_type: relation.subject_type,
            predicate: relation.predicate,
            object: relation.object,
            object_type: relation.object_type,
            confidence: relation.confidence,
            evidence_count: u32::try_from(relation.chunk_indexes.len()).unwrap_or(u32::MAX),
            chunk_ids: relation
                .chunk_indexes
                .iter()
                .map(|index| format!("chunk:{index}"))
                .collect::<Vec<_>>(),
            order_index: Some(relation.order_index),
            metadata: Value::Object(relation.metadata),
        })
        .collect::<Vec<_>>();

    let mut tags = accumulator.tags.into_iter().collect::<Vec<_>>();
    tags.sort();
    let metadata = json!({
        "kind": accumulator.doc_kind.or_else(|| {
            if !accumulator.preferences.is_empty() || !accumulator.decisions.is_empty() {
                Some("profile".to_string())
            } else {
                None
            }
        }),
        "primary_subject": accumulator.primary_subject,
        "decisions": accumulator.decisions.iter().cloned().collect::<Vec<_>>(),
        "preferences": accumulator.preferences.iter().cloned().collect::<Vec<_>>(),
        "extracted_entities": entities.iter().map(|entity| {
            json!({
                "name": entity.name,
                "entity_type": entity.entity_type,
                "aliases": entity.aliases,
            })
        }).collect::<Vec<_>>(),
    });

    log::debug!(
        "[memory:ingestion] parse_document complete title={title:?} \
         entities={} relations={} preferences={} decisions={}",
        entities.len(),
        relations.len(),
        accumulator.preferences.len(),
        accumulator.decisions.len(),
    );

    ParsedIngestion {
        tags,
        metadata,
        entities,
        relations,
        chunk_count: chunks.len(),
        preference_count: accumulator.preferences.len(),
        decision_count: accumulator.decisions.len(),
    }
}

impl UnifiedMemory {
    pub async fn ingest_document(
        &self,
        request: MemoryIngestionRequest,
    ) -> Result<MemoryIngestionResult, String> {
        let parsed = parse_document(
            &request.document.content,
            &request.document.title,
            &request.config,
        )
        .await;
        let (enriched_input, tags) =
            enrich_document_metadata(&request.document, &parsed, &request.config);
        let namespace = Self::sanitize_namespace(&enriched_input.namespace);
        let document_id = self.upsert_document(enriched_input).await?;

        self.upsert_graph_relations(&namespace, &document_id, &parsed, &request.config)
            .await?;

        Ok(MemoryIngestionResult {
            document_id,
            namespace,
            model_name: request.config.model_name,
            extraction_mode: request.config.extraction_mode.as_str().to_string(),
            chunk_count: parsed.chunk_count,
            entity_count: parsed.entities.len(),
            relation_count: parsed.relations.len(),
            preference_count: parsed.preference_count,
            decision_count: parsed.decision_count,
            tags,
            entities: parsed.entities,
            relations: parsed.relations,
        })
    }

    /// Extract entities/relations and write them to the graph for a document
    /// that has already been stored via [`upsert_document`].
    ///
    /// This avoids the redundant second upsert that would happen if the
    /// background ingestion queue called [`ingest_document`] on an already-
    /// persisted document.
    pub async fn extract_graph(
        &self,
        document_id: &str,
        document: &NamespaceDocumentInput,
        config: &MemoryIngestionConfig,
    ) -> Result<MemoryIngestionResult, String> {
        let parsed = parse_document(&document.content, &document.title, config).await;
        let namespace = Self::sanitize_namespace(&document.namespace);

        self.upsert_graph_relations(&namespace, document_id, &parsed, config)
            .await?;

        let (_, tags) = enrich_document_metadata(document, &parsed, config);

        Ok(MemoryIngestionResult {
            document_id: document_id.to_string(),
            namespace,
            model_name: config.model_name.clone(),
            extraction_mode: config.extraction_mode.as_str().to_string(),
            chunk_count: parsed.chunk_count,
            entity_count: parsed.entities.len(),
            relation_count: parsed.relations.len(),
            preference_count: parsed.preference_count,
            decision_count: parsed.decision_count,
            tags,
            entities: parsed.entities,
            relations: parsed.relations,
        })
    }

    /// Clear existing relations for the document then upsert all extracted
    /// relations into the namespace graph.
    async fn upsert_graph_relations(
        &self,
        namespace: &str,
        document_id: &str,
        parsed: &ParsedIngestion,
        config: &MemoryIngestionConfig,
    ) -> Result<(), String> {
        self.graph_remove_document_namespace(namespace, document_id)
            .await?;

        for relation in &parsed.relations {
            let chunk_ids = relation
                .chunk_ids
                .iter()
                .filter_map(|chunk_id| chunk_id.strip_prefix("chunk:"))
                .map(|chunk_index| format!("{document_id}:{chunk_index}"))
                .collect::<Vec<_>>();

            let attrs = json!({
                "source": "ingestion",
                "model_name": config.model_name,
                "extraction_mode": config.extraction_mode.as_str(),
                "confidence": relation.confidence,
                "evidence_count": relation.evidence_count,
                "order_index": relation.order_index,
                "document_id": document_id,
                "document_ids": [document_id],
                "chunk_ids": chunk_ids,
                "entity_types": {
                    "subject": relation.subject_type,
                    "object": relation.object_type,
                },
                "metadata": relation.metadata,
            });

            self.graph_upsert_namespace(
                namespace,
                &relation.subject,
                &relation.predicate,
                &relation.object,
                &attrs,
            )
            .await?;
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "ingestion_tests.rs"]
mod tests;
