//! Types produced by entity extractors (Phase 2 / #708).
//!
//! The pipeline runs one or more [`super::EntityExtractor`] impls over each
//! admitted chunk and collects all their output into [`ExtractedEntities`].

use serde::{Deserialize, Serialize};

/// Classification of an extracted span.
///
/// Split into two categories:
/// - **Mechanical** — regex finds these deterministically. Stable, high precision,
///   limited recall. These are "identifiers" (pointers), not "entities"
///   in the semantic sense.
/// - **Semantic** — model-based (future GLiNER / LLM). Named references to
///   real-world objects: Person, Organization, Location, Event, Product.
///
/// Phase 2 ships with mechanical-only; semantic variants are populated in
/// Phase 3+ either at seal time by the summariser LLM or by a dedicated
/// per-chunk NER step if added later.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EntityKind {
    // Mechanical
    Email,
    Url,
    Handle,
    Hashtag,
    // Semantic — emitted by the LLM extractor.
    Person,
    Organization,
    Location,
    Event,
    Product,
    /// Temporal expressions: "Friday", "Q2 2026", "EOD tomorrow", "next sprint".
    Datetime,
    /// Tools / frameworks / programming languages / services:
    /// "Rust", "OAuth", "Slack API", "nomic-embed".
    Technology,
    /// Code / ticket / doc references that point at something addressable:
    /// "PR #934", "src/openhuman/...", "OH-42", "ab7da2e2".
    Artifact,
    /// Amounts / metrics / money: "$5K", "20/min", "10k tokens", "52 chunks".
    Quantity,
    Misc,
    // Thematic — scorer-surfaced topics (hashtag-like short phrases or
    // LLM-extracted themes). Promoted into the canonical entity stream
    // by the resolver so Phase 3c topic trees can route on themes the
    // same way they route on people/orgs. A chunk saying "Phoenix
    // migration ships Friday" emits `topic:phoenix` and `topic:migration`
    // in addition to any emails/hashtags the mechanical extractors find.
    Topic,
}

impl EntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Url => "url",
            Self::Handle => "handle",
            Self::Hashtag => "hashtag",
            Self::Person => "person",
            Self::Organization => "organization",
            Self::Location => "location",
            Self::Event => "event",
            Self::Product => "product",
            Self::Datetime => "datetime",
            Self::Technology => "technology",
            Self::Artifact => "artifact",
            Self::Quantity => "quantity",
            Self::Misc => "misc",
            Self::Topic => "topic",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "email" => Ok(Self::Email),
            "url" => Ok(Self::Url),
            "handle" => Ok(Self::Handle),
            "hashtag" => Ok(Self::Hashtag),
            "person" => Ok(Self::Person),
            "organization" => Ok(Self::Organization),
            "location" => Ok(Self::Location),
            "event" => Ok(Self::Event),
            "product" => Ok(Self::Product),
            "datetime" => Ok(Self::Datetime),
            "technology" => Ok(Self::Technology),
            "artifact" => Ok(Self::Artifact),
            "quantity" => Ok(Self::Quantity),
            "misc" => Ok(Self::Misc),
            "topic" => Ok(Self::Topic),
            other => Err(format!("unknown entity kind: {other}")),
        }
    }

    /// Whether this kind comes from deterministic extraction.
    pub fn is_mechanical(self) -> bool {
        matches!(self, Self::Email | Self::Url | Self::Handle | Self::Hashtag)
    }
}

/// One extracted span from a chunk's content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtractedEntity {
    pub kind: EntityKind,
    /// Surface form as it appears in the chunk.
    pub text: String,
    /// Character offsets `[start, end)` into the chunk text.
    pub span_start: u32,
    pub span_end: u32,
    /// Extractor confidence `[0.0, 1.0]`. Regex = 1.0; model-based = output.
    pub score: f32,
}

/// Topic candidate (hashtag-style or summariser-labeled).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtractedTopic {
    /// Normalised topic text (lowercase, no leading `#`).
    pub label: String,
    pub score: f32,
}

/// Aggregate output of one or more extractors on a single chunk.
///
/// `llm_importance` and `llm_importance_reason` are populated by extractors
/// that piggyback an importance rating on their NER call (see
/// [`super::llm::LlmEntityExtractor`]). Cheap regex extractors leave them
/// `None`; downstream signal compute treats `None` as "no LLM signal" and
/// the weighted combine zeroes that contribution out so behaviour matches
/// pre-LLM Phase 2 exactly when LLM is disabled.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtractedEntities {
    pub entities: Vec<ExtractedEntity>,
    pub topics: Vec<ExtractedTopic>,
    /// Optional LLM-rated importance in `[0.0, 1.0]` for this chunk.
    /// `None` means no LLM signal is available.
    #[serde(default)]
    pub llm_importance: Option<f32>,
    /// One-line audit trail from the LLM explaining the importance rating.
    /// Used purely for diagnostics; never feeds back into scoring.
    #[serde(default)]
    pub llm_importance_reason: Option<String>,
}

impl ExtractedEntities {
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty() && self.topics.is_empty()
    }

    /// Count of unique `(kind, text)` pairs, case-insensitive. Used as a scoring signal.
    pub fn unique_entity_count(&self) -> usize {
        use std::collections::BTreeSet;
        self.entities
            .iter()
            .map(|e| (e.kind, e.text.to_lowercase()))
            .collect::<BTreeSet<_>>()
            .len()
    }

    /// Merge another extractor's output into this one.
    ///
    /// Deduplicates entities by `(kind, normalised_text, span_start)` and
    /// topics by `label` so the same match from two extractors doesn't get
    /// double-counted.
    ///
    /// LLM importance signals merge by **maximum** — if either side rated
    /// the chunk as important, the merged result keeps that higher rating.
    /// The reason from whichever side won the max wins; if they tied or
    /// both are absent, the non-empty one (if any) is kept.
    pub fn merge(&mut self, other: ExtractedEntities) {
        use std::collections::BTreeSet;
        let mut seen: BTreeSet<(EntityKind, String, u32)> = self
            .entities
            .iter()
            .map(|e| (e.kind, e.text.to_lowercase(), e.span_start))
            .collect();
        for e in other.entities {
            let key = (e.kind, e.text.to_lowercase(), e.span_start);
            if seen.insert(key) {
                self.entities.push(e);
            }
        }
        let mut topic_seen: BTreeSet<String> =
            self.topics.iter().map(|t| t.label.clone()).collect();
        for t in other.topics {
            if topic_seen.insert(t.label.clone()) {
                self.topics.push(t);
            }
        }

        // Merge LLM importance: max wins, reason follows the max.
        match (self.llm_importance, other.llm_importance) {
            (Some(a), Some(b)) if b > a => {
                self.llm_importance = Some(b);
                self.llm_importance_reason = other.llm_importance_reason;
            }
            (None, Some(b)) => {
                self.llm_importance = Some(b);
                self.llm_importance_reason = other.llm_importance_reason;
            }
            // self.a >= other.b OR other has nothing — keep self
            _ => {
                if self.llm_importance_reason.is_none() {
                    self.llm_importance_reason = other.llm_importance_reason;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_kind_round_trip() {
        for k in [
            EntityKind::Email,
            EntityKind::Url,
            EntityKind::Handle,
            EntityKind::Hashtag,
            EntityKind::Person,
            EntityKind::Organization,
            EntityKind::Location,
            EntityKind::Event,
            EntityKind::Product,
            EntityKind::Datetime,
            EntityKind::Technology,
            EntityKind::Artifact,
            EntityKind::Quantity,
            EntityKind::Misc,
            EntityKind::Topic,
        ] {
            assert_eq!(EntityKind::parse(k.as_str()).unwrap(), k);
        }
    }

    #[test]
    fn mechanical_classification() {
        assert!(EntityKind::Email.is_mechanical());
        assert!(EntityKind::Url.is_mechanical());
        assert!(EntityKind::Handle.is_mechanical());
        assert!(EntityKind::Hashtag.is_mechanical());
        assert!(!EntityKind::Person.is_mechanical());
    }

    #[test]
    fn unique_entity_count_dedups_case_insensitive() {
        let e = ExtractedEntities {
            entities: vec![
                ExtractedEntity {
                    kind: EntityKind::Person,
                    text: "Alice".into(),
                    span_start: 0,
                    span_end: 5,
                    score: 1.0,
                },
                ExtractedEntity {
                    kind: EntityKind::Person,
                    text: "alice".into(),
                    span_start: 10,
                    span_end: 15,
                    score: 1.0,
                },
            ],
            topics: vec![],
            llm_importance: None,
            llm_importance_reason: None,
        };
        assert_eq!(e.unique_entity_count(), 1);
    }

    #[test]
    fn unique_entity_count_keeps_different_kinds_distinct() {
        let e = ExtractedEntities {
            entities: vec![
                ExtractedEntity {
                    kind: EntityKind::Handle,
                    text: "alice".into(),
                    span_start: 0,
                    span_end: 5,
                    score: 1.0,
                },
                ExtractedEntity {
                    kind: EntityKind::Hashtag,
                    text: "alice".into(),
                    span_start: 10,
                    span_end: 15,
                    score: 1.0,
                },
            ],
            topics: vec![],
            llm_importance: None,
            llm_importance_reason: None,
        };
        assert_eq!(e.unique_entity_count(), 2);
    }

    #[test]
    fn merge_dedups_by_kind_text_span() {
        let mut a = ExtractedEntities {
            entities: vec![ExtractedEntity {
                kind: EntityKind::Email,
                text: "x@y.com".into(),
                span_start: 0,
                span_end: 7,
                score: 1.0,
            }],
            topics: vec![],
            llm_importance: None,
            llm_importance_reason: None,
        };
        let b = ExtractedEntities {
            entities: vec![
                ExtractedEntity {
                    kind: EntityKind::Email,
                    text: "x@y.com".into(),
                    span_start: 0,
                    span_end: 7,
                    score: 1.0,
                }, // dup
                ExtractedEntity {
                    kind: EntityKind::Email,
                    text: "x@y.com".into(),
                    span_start: 50,
                    span_end: 57,
                    score: 1.0,
                }, // different span — keep
            ],
            topics: vec![],
            llm_importance: None,
            llm_importance_reason: None,
        };
        a.merge(b);
        assert_eq!(a.entities.len(), 2);
    }
}
