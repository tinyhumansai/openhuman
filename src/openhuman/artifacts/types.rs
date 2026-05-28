use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The category of an artifact produced by the agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactKind {
    Presentation,
    Document,
    Image,
    Other,
}

impl Default for ArtifactKind {
    fn default() -> Self {
        Self::Other
    }
}

impl ArtifactKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Presentation => "presentation",
            Self::Document => "document",
            Self::Image => "image",
            Self::Other => "other",
        }
    }

    /// Parse a raw string into an `ArtifactKind`. Case-insensitive; unknown
    /// values fall back to `Other`.
    pub fn parse(raw: &str) -> Self {
        match raw.to_ascii_lowercase().as_str() {
            "presentation" => Self::Presentation,
            "document" => Self::Document,
            "image" => Self::Image,
            _ => Self::Other,
        }
    }
}

/// Lifecycle status of an artifact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactStatus {
    Pending,
    Ready,
    Failed,
}

impl Default for ArtifactStatus {
    fn default() -> Self {
        Self::Pending
    }
}

impl ArtifactStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }

    /// Parse a raw string into an `ArtifactStatus`. Case-insensitive; unknown
    /// values fall back to `Pending`.
    pub fn parse(raw: &str) -> Self {
        match raw.to_ascii_lowercase().as_str() {
            "ready" => Self::Ready,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }
}

/// Metadata record for a single agent-generated artifact.
///
/// Persisted as `<workspace>/artifacts/<id>/meta.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactMeta {
    /// Unique artifact identifier (UUID string).
    pub id: String,
    /// Category of the artifact.
    pub kind: ArtifactKind,
    /// Human-readable title.
    pub title: String,
    /// Relative path from the artifacts root, e.g. `"<uuid>/deck.pptx"`.
    pub path: String,
    /// Artifact file size in bytes.
    pub size_bytes: u64,
    /// Current lifecycle status.
    pub status: ArtifactStatus,
    /// UTC timestamp when this artifact was created.
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    // ── ArtifactKind ───────────────────────────────────────────────────────────

    #[test]
    fn artifact_kind_default_is_other() {
        assert_eq!(ArtifactKind::default(), ArtifactKind::Other);
    }

    #[test]
    fn artifact_kind_as_str_roundtrip() {
        assert_eq!(ArtifactKind::Presentation.as_str(), "presentation");
        assert_eq!(ArtifactKind::Document.as_str(), "document");
        assert_eq!(ArtifactKind::Image.as_str(), "image");
        assert_eq!(ArtifactKind::Other.as_str(), "other");
    }

    #[test]
    fn artifact_kind_parse_case_insensitive() {
        assert_eq!(
            ArtifactKind::parse("presentation"),
            ArtifactKind::Presentation
        );
        assert_eq!(
            ArtifactKind::parse("PRESENTATION"),
            ArtifactKind::Presentation
        );
        assert_eq!(ArtifactKind::parse("Document"), ArtifactKind::Document);
        assert_eq!(ArtifactKind::parse("IMAGE"), ArtifactKind::Image);
        assert_eq!(ArtifactKind::parse("other"), ArtifactKind::Other);
        assert_eq!(ArtifactKind::parse("unknown"), ArtifactKind::Other);
        assert_eq!(ArtifactKind::parse(""), ArtifactKind::Other);
    }

    #[test]
    fn artifact_kind_serde_roundtrip() {
        for kind in [
            ArtifactKind::Presentation,
            ArtifactKind::Document,
            ArtifactKind::Image,
            ArtifactKind::Other,
        ] {
            let json = serde_json::to_value(&kind).unwrap();
            let back: ArtifactKind = serde_json::from_value(json).unwrap();
            assert_eq!(back, kind);
        }
    }

    #[test]
    fn artifact_kind_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&ArtifactKind::Presentation).unwrap(),
            "\"presentation\""
        );
        assert_eq!(
            serde_json::to_string(&ArtifactKind::Document).unwrap(),
            "\"document\""
        );
    }

    // ── ArtifactStatus ─────────────────────────────────────────────────────────

    #[test]
    fn artifact_status_default_is_pending() {
        assert_eq!(ArtifactStatus::default(), ArtifactStatus::Pending);
    }

    #[test]
    fn artifact_status_as_str_roundtrip() {
        assert_eq!(ArtifactStatus::Pending.as_str(), "pending");
        assert_eq!(ArtifactStatus::Ready.as_str(), "ready");
        assert_eq!(ArtifactStatus::Failed.as_str(), "failed");
    }

    #[test]
    fn artifact_status_parse_case_insensitive() {
        assert_eq!(ArtifactStatus::parse("pending"), ArtifactStatus::Pending);
        assert_eq!(ArtifactStatus::parse("READY"), ArtifactStatus::Ready);
        assert_eq!(ArtifactStatus::parse("Failed"), ArtifactStatus::Failed);
        assert_eq!(ArtifactStatus::parse("unknown"), ArtifactStatus::Pending);
        assert_eq!(ArtifactStatus::parse(""), ArtifactStatus::Pending);
    }

    #[test]
    fn artifact_status_serde_roundtrip() {
        for status in [
            ArtifactStatus::Pending,
            ArtifactStatus::Ready,
            ArtifactStatus::Failed,
        ] {
            let json = serde_json::to_value(&status).unwrap();
            let back: ArtifactStatus = serde_json::from_value(json).unwrap();
            assert_eq!(back, status);
        }
    }

    // ── ArtifactMeta ───────────────────────────────────────────────────────────

    #[test]
    fn artifact_meta_serde_roundtrip() {
        let meta = ArtifactMeta {
            id: "abc-123".to_string(),
            kind: ArtifactKind::Presentation,
            title: "Q3 Deck".to_string(),
            path: "abc-123/deck.pptx".to_string(),
            size_bytes: 204800,
            status: ArtifactStatus::Ready,
            created_at: Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap(),
        };
        let json = serde_json::to_value(&meta).unwrap();
        assert_eq!(json["id"], "abc-123");
        assert_eq!(json["kind"], "presentation");
        assert_eq!(json["status"], "ready");
        let back: ArtifactMeta = serde_json::from_value(json).unwrap();
        assert_eq!(back.id, meta.id);
        assert_eq!(back.kind, meta.kind);
        assert_eq!(back.status, meta.status);
        assert_eq!(back.size_bytes, meta.size_bytes);
    }

    #[test]
    fn artifact_meta_json_shape() {
        let meta = ArtifactMeta {
            id: "x".to_string(),
            kind: ArtifactKind::Other,
            title: "test".to_string(),
            path: "x/file.txt".to_string(),
            size_bytes: 0,
            status: ArtifactStatus::Pending,
            created_at: Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(),
        };
        let v = serde_json::to_value(&meta).unwrap();
        // Verify all expected fields are present
        assert!(v.get("id").is_some());
        assert!(v.get("kind").is_some());
        assert!(v.get("title").is_some());
        assert!(v.get("path").is_some());
        assert!(v.get("size_bytes").is_some());
        assert!(v.get("status").is_some());
        assert!(v.get("created_at").is_some());
    }

    #[test]
    fn artifact_meta_missing_field_deserializes_error() {
        // Ensure missing required fields cause a deserialization error
        let incomplete = json!({ "id": "x", "kind": "other" });
        let result: Result<ArtifactMeta, _> = serde_json::from_value(incomplete);
        assert!(result.is_err());
    }
}
