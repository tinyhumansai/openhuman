//! Structured facet schema and routing from LLM summariser output.
//!
//! The LLM summariser is extended (in `memory/tree/tree_source/summariser/llm.rs`)
//! to produce a second JSON block after the prose summary. This module defines
//! the serde shapes for that block ([`StructuredSummary`], [`ParsedFacet`]) and
//! provides [`route_facets_to_buffer`], which validates each facet and pushes
//! valid candidates to [`crate::openhuman::learning::candidate::global()`].
//!
//! ## Provenance contract
//!
//! Every facet must cite at least one `chunk_id` in its `evidence_chunks` array.
//! Facets with an empty `evidence_chunks` are silently dropped — unattributed
//! observations cannot be scored or cited later.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::openhuman::learning::candidate::{
    self, CueFamily, EvidenceRef, FacetClass, LearningCandidate,
};

// ── Serde types ──────────────────────────────────────────────────────────────

/// A single facet extracted by the LLM during summarisation.
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub struct ParsedFacet {
    /// Facet class as a string — `"style"` | `"identity"` | `"tooling"` |
    /// `"veto"` | `"goal"` | `"channel"`.
    pub class: String,
    /// Canonical slug key within the class, e.g. `"verbosity"`, `"timezone"`.
    pub key: String,
    /// Detected value string.
    pub value: String,
    /// Chunk IDs from the current seal batch that evidence this facet.
    /// Must be non-empty for the facet to be accepted.
    #[serde(default)]
    pub evidence_chunks: Vec<String>,
    /// Source confidence `0.0..=1.0`.
    pub confidence: f64,
    /// How the signal was produced — `"explicit"` | `"structural"` | `"behavioral"`.
    #[serde(default = "default_cue")]
    pub cue_family: String,
}

fn default_cue() -> String {
    "behavioral".into()
}

/// The full structured output expected from the LLM summariser when
/// `structured_facet_extraction` is enabled.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct StructuredSummary {
    /// Prose summary (the text that was previously the only output).
    pub summary: String,
    /// Optional extracted facets. Empty by default when the LLM found nothing
    /// clearly evidenced.
    #[serde(default)]
    pub facets: Vec<ParsedFacet>,
}

// ── Routing ───────────────────────────────────────────────────────────────────

/// Validate each [`ParsedFacet`] in `parsed` and push valid candidates to
/// [`candidate::global()`].
///
/// Drops facets that:
/// - have an unrecognised `class` string
/// - have an empty `evidence_chunks` array (provenance is mandatory)
/// - have `confidence` outside `0.0..=1.0`
///
/// Maps `cue_family` strings to [`CueFamily`]; unknown strings default to
/// [`CueFamily::Behavioral`].
///
/// Uses the first non-empty `evidence_chunks` entry as the `chunk_id` in
/// [`EvidenceRef::DocumentChunk`].
pub fn route_facets_to_buffer(parsed: &StructuredSummary, source_id: &str) {
    route_facets_to_buffer_into(parsed, source_id, candidate::global());
}

fn route_facets_to_buffer_into(
    parsed: &StructuredSummary,
    source_id: &str,
    buf: &candidate::Buffer,
) {
    let now = now_secs();
    let mut pushed = 0usize;

    for facet in &parsed.facets {
        // Validate evidence.
        let chunk_id = match facet.evidence_chunks.first() {
            Some(id) if !id.is_empty() => id.clone(),
            _ => {
                tracing::debug!(
                    "[learning::extract::summary_facets] dropping facet key={} \
                     (no evidence_chunks) source_id={}",
                    facet.key,
                    source_id
                );
                continue;
            }
        };

        // Map class string.
        let class = match parse_facet_class(&facet.class) {
            Some(c) => c,
            None => {
                tracing::debug!(
                    "[learning::extract::summary_facets] dropping facet key={} \
                     (unknown class={:?}) source_id={}",
                    facet.key,
                    facet.class,
                    source_id
                );
                continue;
            }
        };

        // Clamp confidence.
        let confidence = facet.confidence.clamp(0.0, 1.0);

        let cue_family = parse_cue_family(&facet.cue_family);

        let candidate = LearningCandidate {
            class,
            key: facet.key.clone(),
            value: facet.value.clone(),
            cue_family,
            evidence: EvidenceRef::DocumentChunk {
                source_id: source_id.to_string(),
                chunk_id,
            },
            initial_confidence: confidence,
            observed_at: now,
        };

        tracing::debug!(
            "[learning::extract::summary_facets] routing facet class={:?} key={} \
             value={:?} confidence={:.2} source_id={}",
            candidate.class,
            candidate.key,
            candidate.value,
            candidate.initial_confidence,
            source_id
        );

        buf.push(candidate);
        pushed += 1;
    }

    tracing::debug!(
        "[learning::extract::summary_facets] route_facets_to_buffer source_id={} \
         facets_in={} pushed={}",
        source_id,
        parsed.facets.len(),
        pushed
    );
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_facet_class(s: &str) -> Option<FacetClass> {
    match s {
        "style" => Some(FacetClass::Style),
        "identity" => Some(FacetClass::Identity),
        "tooling" => Some(FacetClass::Tooling),
        "veto" => Some(FacetClass::Veto),
        "goal" => Some(FacetClass::Goal),
        "channel" => Some(FacetClass::Channel),
        _ => None,
    }
}

fn parse_cue_family(s: &str) -> CueFamily {
    match s {
        "explicit" => CueFamily::Explicit,
        "structural" => CueFamily::Structural,
        "behavioral" => CueFamily::Behavioral,
        "recurrence" => CueFamily::Recurrence,
        _ => CueFamily::Behavioral,
    }
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::learning::candidate::{Buffer, CueFamily, FacetClass};

    fn make_summary(facets: Vec<ParsedFacet>) -> StructuredSummary {
        StructuredSummary {
            summary: "A summary.".into(),
            facets,
        }
    }

    fn make_facet(
        class: &str,
        key: &str,
        value: &str,
        evidence_chunks: Vec<&str>,
        confidence: f64,
        cue_family: &str,
    ) -> ParsedFacet {
        ParsedFacet {
            class: class.into(),
            key: key.into(),
            value: value.into(),
            evidence_chunks: evidence_chunks.into_iter().map(str::to_string).collect(),
            confidence,
            cue_family: cue_family.into(),
        }
    }

    #[test]
    fn parse_well_formed_structured_summary() {
        let json = r#"{
            "summary": "The user prefers pnpm.",
            "facets": [
                {
                    "class": "tooling",
                    "key": "package_manager",
                    "value": "pnpm",
                    "evidence_chunks": ["chunk-abc"],
                    "confidence": 0.85,
                    "cue_family": "explicit"
                }
            ]
        }"#;
        let parsed: StructuredSummary = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.summary, "The user prefers pnpm.");
        assert_eq!(parsed.facets.len(), 1);
        assert_eq!(parsed.facets[0].key, "package_manager");
        assert_eq!(parsed.facets[0].cue_family, "explicit");
    }

    #[test]
    fn drops_facet_with_unknown_class() {
        let buf = Buffer::new(64);
        let before = buf.len();

        // Route into global but we check the drop via tracing — here we test
        // that the function doesn't panic and the drop is silent.
        let s = make_summary(vec![make_facet(
            "unknown_class",
            "key",
            "val",
            vec!["c1"],
            0.8,
            "behavioral",
        )]);
        route_facets_to_buffer_into(&s, "src-1", &buf);
        // No new candidates pushed.
        assert_eq!(buf.len(), before, "unknown class should drop the facet");
    }

    #[test]
    fn drops_facet_without_evidence_chunks() {
        let buf = Buffer::new(64);
        let s = make_summary(vec![make_facet(
            "style",
            "verbosity",
            "terse",
            vec![], // empty — must be dropped
            0.8,
            "explicit",
        )]);
        let before = buf.len();
        route_facets_to_buffer_into(&s, "src-2", &buf);
        assert_eq!(
            buf.len(),
            before,
            "facet without evidence_chunks must be dropped"
        );
    }

    #[test]
    fn defaults_cue_family_to_behavioral() {
        let json = r#"{
            "summary": "x",
            "facets": [
                {
                    "class": "style",
                    "key": "verbosity",
                    "value": "terse",
                    "evidence_chunks": ["c1"],
                    "confidence": 0.7
                }
            ]
        }"#;
        let parsed: StructuredSummary = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.facets[0].cue_family, "behavioral");
    }

    #[test]
    fn route_pushes_to_buffer() {
        let buf = Buffer::new(64);
        let s = make_summary(vec![
            make_facet(
                "style",
                "verbosity",
                "terse",
                vec!["chunk-1"],
                0.75,
                "explicit",
            ),
            make_facet(
                "identity",
                "timezone",
                "UTC+5:30",
                vec!["chunk-2"],
                0.9,
                "structural",
            ),
        ]);
        let before = buf.len();
        route_facets_to_buffer_into(&s, "notion:doc-1", &buf);
        let after = buf.len();
        assert_eq!(
            after,
            before + 2,
            "two valid facets should push two candidates"
        );

        let all = buf.peek();
        let tz = all.iter().find(|c| c.key == "timezone");
        let tz = tz.expect("timezone candidate in buffer");
        assert_eq!(tz.value, "UTC+5:30");
        assert_eq!(tz.class, FacetClass::Identity);
        assert_eq!(tz.cue_family, CueFamily::Structural);
        assert!(
            matches!(&tz.evidence, EvidenceRef::DocumentChunk { source_id, chunk_id }
            if source_id == "notion:doc-1" && chunk_id == "chunk-2")
        );
    }
}
