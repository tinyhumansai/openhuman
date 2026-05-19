//! Standalone documents -> canonical Markdown.
//!
//! Document sources are single-record (no grouping): one Notion page, one
//! Drive doc, one meeting-note file. The canonicaliser trims and passes through the body as canonical Markdown. If the body is already markdown it is kept verbatim, and provider/title metadata stays in front-matter rather than a generated heading.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

use super::{normalize_source_ref, CanonicalisedSource};
use crate::openhuman::memory::tree::types::{Metadata, SourceKind};

/// Adapter input for a single document.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DocumentInput {
    /// Provider name (e.g. `notion`, `drive`, `meeting_notes`).
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Document title.
    pub title: String,
    /// Document body (markdown preferred; plain text also accepted).
    pub body: String,
    /// When the document was last modified at the source.
    #[serde(
        default = "default_modified_at",
        deserialize_with = "deserialize_modified_at"
    )]
    pub modified_at: DateTime<Utc>,
    /// Optional pointer back to source (URL, file path, Notion page id).
    #[serde(default)]
    pub source_ref: Option<String>,
}

fn default_provider() -> String {
    "unknown".to_string()
}

fn default_modified_at() -> DateTime<Utc> {
    Utc::now()
}

fn deserialize_modified_at<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum ModifiedAtValue {
        Milliseconds(i64),
        Iso8601(String),
    }

    match ModifiedAtValue::deserialize(deserializer)? {
        ModifiedAtValue::Milliseconds(value) => DateTime::<Utc>::from_timestamp_millis(value)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid unix timestamp in milliseconds: {value}"))),
        ModifiedAtValue::Iso8601(value) => chrono::DateTime::parse_from_rfc3339(&value)
            .map(|parsed| parsed.with_timezone(&Utc))
            .map_err(|err| serde::de::Error::custom(format!("invalid RFC3339 timestamp: {err}"))),
    }
}

/// Canonicalise a single document into a [`CanonicalisedSource`]. Returns
/// `Ok(None)` if both the title and body are empty; caller treats that as nothing
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
    // No leading `# provider - title` header. Provider / title info
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
        // No leading `# notion - Launch plan` header - that info belongs in front-matter.
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

    #[test]
    fn deserializes_epoch_millis_modified_at() {
        let raw = serde_json::json!({
            "provider": "drive",
            "title": "Spec",
            "body": "Notes",
            "modified_at": 1_700_000_000_000i64,
        });

        let parsed: DocumentInput = serde_json::from_value(raw).unwrap();

        assert_eq!(parsed.provider, "drive");
        assert_eq!(
            parsed.modified_at,
            Utc.timestamp_millis_opt(1_700_000_000_000).unwrap()
        );
    }

    #[test]
    fn deserializes_iso_8601_modified_at() {
        let raw = serde_json::json!({
            "provider": "drive",
            "title": "Spec",
            "body": "Notes",
            "modified_at": "2026-05-17T19:30:00Z",
        });

        let parsed: DocumentInput = serde_json::from_value(raw).unwrap();

        assert_eq!(
            parsed.modified_at,
            chrono::DateTime::parse_from_rfc3339("2026-05-17T19:30:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn missing_provider_defaults_to_unknown() {
        let raw = serde_json::json!({
            "title": "Spec",
            "body": "Notes",
            "modified_at": 1_700_000_000_000i64,
        });

        let parsed: DocumentInput = serde_json::from_value(raw).unwrap();

        assert_eq!(parsed.provider, "unknown");
    }

    #[test]
    fn missing_modified_at_defaults_to_nowish() {
        let before = Utc::now();
        let raw = serde_json::json!({
            "provider": "drive",
            "title": "Spec",
            "body": "Notes",
        });

        let parsed: DocumentInput = serde_json::from_value(raw).unwrap();
        let after = Utc::now();

        assert!(parsed.modified_at >= before);
        assert!(parsed.modified_at <= after);
    }

}




