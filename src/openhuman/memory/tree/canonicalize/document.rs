//! Standalone documents → canonical Markdown.
//!
//! Document sources are single-record (no grouping): one Notion page, one
//! Drive doc, one meeting-note file. The canonicaliser adds a small title
//! header and passes through the body; if the body is already markdown it
//! is kept verbatim.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{normalize_source_ref, CanonicalisedSource};
use crate::openhuman::memory::tree::types::{Metadata, SourceKind};

/// Adapter input for a single document.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DocumentInput {
    /// Provider name (e.g. `notion`, `drive`, `meeting_notes`).
    pub provider: String,
    /// Document title.
    pub title: String,
    /// Document body (markdown preferred; plain text also accepted).
    pub body: String,
    /// When the document was last modified at the source.
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub modified_at: DateTime<Utc>,
    /// Optional pointer back to source (URL, file path, Notion page id).
    #[serde(default)]
    pub source_ref: Option<String>,
}

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
    md.push_str(&format!("# {} — {}\n\n", doc.provider, doc.title));
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
    fn renders_title_and_body() {
        let out = canonicalise(
            "d1",
            "alice",
            &[],
            doc("Launch plan", "step one\n\nstep two"),
        )
        .unwrap()
        .unwrap();
        assert!(out.markdown.starts_with("# notion — Launch plan\n\n"));
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
}
