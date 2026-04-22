//! Entity extraction (Phase 2 / #708).
//!
//! Exposes [`EntityExtractor`] as a pluggable interface and a default
//! [`CompositeExtractor`] that runs a chain of extractors and merges their
//! output. Phase 2 ships with the mechanical regex extractor only; semantic
//! NER (GLiNER / LLM) plugs in later without changing any call sites.

mod extractor;
pub mod llm;
pub mod regex;
pub mod types;

pub use extractor::{CompositeExtractor, EntityExtractor, RegexEntityExtractor};
pub use llm::{LlmEntityExtractor, LlmExtractorConfig};
pub use types::{EntityKind, ExtractedEntities, ExtractedEntity, ExtractedTopic};
