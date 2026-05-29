//! Standalone documents → canonical Markdown.
//!
//! Document sources are single-record (no grouping): one Notion page, one
//! Drive doc, one meeting-note file. The canonicaliser adds a small title
//! header and passes through the body; if the body is already markdown it
//! is kept verbatim.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

use super::{normalize_source_ref, CanonicalisedSource};
use crate::openhuman::memory_store::chunks::types::{Metadata, SourceKind};

// ── Serde helpers ─────────────────────────────────────────────────────────────

fn default_provider() -> String {
    "unknown".to_string()
}

fn now_utc() -> DateTime<Utc> {
    Utc::now()
}

/// Deserialise a `DateTime<Utc>` from either:
/// - a JSON integer = epoch **milliseconds** (legacy callers — back-compat),
/// - a JSON string = RFC 3339 / ISO-8601 (e.g. `"2026-05-17T19:30:00Z"`), or
///   a decimal string containing epoch milliseconds.
///
/// On an unparseable string a serde error is returned (no silent default).
fn deserialize_flexible_timestamp<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    /// Untagged helper so serde tries each variant in order.
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RawTs {
        Millis(i64),
        Text(String),
    }

    let raw = RawTs::deserialize(deserializer)?;
    match raw {
        RawTs::Millis(ms) => {
            tracing::debug!("[memory][document] parsed modified_at as epoch-ms: {ms}");
            chrono::TimeZone::timestamp_millis_opt(&Utc, ms)
                .single()
                .ok_or_else(|| serde::de::Error::custom(format!("invalid epoch-ms: {ms}")))
        }
        RawTs::Text(s) => {
            // Try RFC 3339 / ISO-8601 first.
            if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
                tracing::debug!("[memory][document] parsed modified_at as ISO-8601 string: {s}");
                return Ok(dt.with_timezone(&Utc));
            }
            // Fall back: numeric string = epoch milliseconds.
            if let Ok(ms) = s.parse::<i64>() {
                tracing::debug!(
                    "[memory][document] parsed modified_at as numeric-string epoch-ms: {s}"
                );
                return chrono::TimeZone::timestamp_millis_opt(&Utc, ms)
                    .single()
                    .ok_or_else(|| {
                        serde::de::Error::custom(format!("invalid epoch-ms string: {s}"))
                    });
            }
            Err(serde::de::Error::custom(format!(
                "modified_at: cannot parse '{s}' as RFC 3339 or epoch-ms"
            )))
        }
    }
}

// ── Input struct ──────────────────────────────────────────────────────────────

/// Adapter input for a single document.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DocumentInput {
    /// Provider name (e.g. `notion`, `drive`, `meeting_notes`).
    /// Defaults to `"unknown"` when absent (fixes CORE-31).
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Document title.
    pub title: String,
    /// Document body (markdown preferred; plain text also accepted).
    pub body: String,
    /// When the document was last modified at the source.
    ///
    /// Accepts epoch-milliseconds integer (back-compat), RFC 3339 / ISO-8601
    /// string (fixes CORE-2K), or absent → `Utc::now()` (fixes CORE-2J).
    #[serde(
        default = "now_utc",
        deserialize_with = "deserialize_flexible_timestamp"
    )]
    pub modified_at: DateTime<Utc>,
    /// Optional pointer back to source (URL, file path, Notion page id).
    #[serde(default)]
    pub source_ref: Option<String>,
}

/// Canonicalise a single document into a [`CanonicalisedSource`]. Returns
/// `Ok(None)` if both the title and body are empty — caller treats as nothing
/// to ingest.
pub fn canonicalise(
    source_id: &str,
    owner: &str,
    tags: &[String],
    doc: DocumentInput,
) -> Result<Option<CanonicalisedSource>, String> {
    if doc.body.trim().is_empty() && doc.title.trim().is_empty() {
        return Ok(None);
    }

    let mut md = String::new();
    // No leading `# provider — title` header. Provider / title info
    // belongs in the MD front-matter (Phase MD-content).
    md.push_str(doc.body.trim());
    md.push('\n');

    Ok(Some(CanonicalisedSource {
        markdown: md,
        metadata: Metadata {
            source_kind: SourceKind::Document,
            source_id: source_id.to_string(),
            owner: owner.to_string(),
            timestamp: doc.modified_at,
            time_range: (doc.modified_at, doc.modified_at),
            tags: tags.to_vec(),
            source_ref: normalize_source_ref(doc.source_ref),
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn doc(title: &str, body: &str) -> DocumentInput {
        DocumentInput {
            provider: "notion".into(),
            title: title.into(),
            body: body.into(),
            modified_at: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
            source_ref: Some("notion://page/abc".into()),
        }
    }

    #[test]
    fn empty_doc_returns_none() {
        let d = DocumentInput {
            provider: "notion".into(),
            title: "".into(),
            body: "   \n  ".into(),
            modified_at: Utc::now(),
            source_ref: None,
        };
        assert!(canonicalise("d1", "alice", &[], d).unwrap().is_none());
    }

    #[test]
    fn renders_body_without_header() {
        let out = canonicalise(
            "d1",
            "alice",
            &[],
            doc("Launch plan", "step one\n\nstep two"),
        )
        .unwrap()
        .unwrap();
        // No leading `# notion — Launch plan` header — that info belongs in front-matter.
        assert!(
            !out.markdown.starts_with("# "),
            "canonical document MD must NOT start with a `# ` header"
        );
        assert!(out.markdown.contains("step one"));
        assert!(out.markdown.contains("step two"));
    }

    #[test]
    fn metadata_single_point_time_range() {
        let out = canonicalise("d1", "alice", &[], doc("x", "y"))
            .unwrap()
            .unwrap();
        assert_eq!(out.metadata.time_range.0, out.metadata.time_range.1);
        assert_eq!(out.metadata.source_kind, SourceKind::Document);
    }

    #[test]
    fn source_ref_carried_through() {
        let out = canonicalise("d1", "alice", &["proj".into()], doc("x", "y"))
            .unwrap()
            .unwrap();
        assert_eq!(
            out.metadata.source_ref.as_ref().unwrap().value,
            "notion://page/abc"
        );
        assert_eq!(out.metadata.tags, vec!["proj"]);
    }

    #[test]
    fn blank_source_ref_is_dropped() {
        let mut input = doc("x", "y");
        input.source_ref = Some(" \n ".into());
        let out = canonicalise("d1", "alice", &[], input).unwrap().unwrap();
        assert!(out.metadata.source_ref.is_none());
    }

    // ── Serde regression / fix tests (CORE-2K / CORE-2J / CORE-31) ───────────

    /// Regression: existing callers send epoch-ms as a JSON integer — must still work.
    #[test]
    fn modified_at_epoch_ms_integer_still_works() {
        let json = r#"{
            "provider": "notion",
            "title": "My doc",
            "body": "content",
            "modified_at": 1700000000000
        }"#;
        let input: DocumentInput =
            serde_json::from_str(json).expect("epoch-ms integer should parse");
        assert_eq!(
            input.modified_at.timestamp_millis(),
            1_700_000_000_000,
            "epoch-ms round-trip"
        );
    }

    /// Fix CORE-2K: callers sending an ISO-8601 string must be accepted.
    #[test]
    fn modified_at_iso8601_string_accepted() {
        let json = r#"{
            "provider": "drive",
            "title": "Meeting notes",
            "body": "agenda here",
            "modified_at": "2026-05-17T19:30:00Z"
        }"#;
        let input: DocumentInput =
            serde_json::from_str(json).expect("ISO-8601 string should parse");
        assert_eq!(input.modified_at.timestamp(), 1_779_046_200);
    }

    /// Fix CORE-2J: omitting modified_at must default to approximately now (within 5 s).
    #[test]
    fn modified_at_missing_defaults_to_now() {
        let before = Utc::now();
        let json = r#"{
            "provider": "notion",
            "title": "No timestamp doc",
            "body": "body text"
        }"#;
        let input: DocumentInput =
            serde_json::from_str(json).expect("missing modified_at should parse");
        let after = Utc::now();
        assert!(
            input.modified_at >= before && input.modified_at <= after,
            "default modified_at {ts} must fall between {before} and {after}",
            ts = input.modified_at,
        );
    }

    /// Fix CORE-31: omitting provider must default to "unknown".
    #[test]
    fn provider_missing_defaults_to_unknown() {
        let json = r#"{
            "title": "No provider doc",
            "body": "body text",
            "modified_at": 1700000000000
        }"#;
        let input: DocumentInput =
            serde_json::from_str(json).expect("missing provider should parse");
        assert_eq!(input.provider, "unknown");
    }
}
