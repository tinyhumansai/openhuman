use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The category of an artifact produced by the agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum ArtifactKind {
    Presentation,
    Document,
    Image,
    Video,
    #[default]
    Other,
}

impl ArtifactKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Presentation => "presentation",
            Self::Document => "document",
            Self::Image => "image",
            Self::Video => "video",
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
            "video" => Self::Video,
            _ => Self::Other,
        }
    }
}

/// Lifecycle status of an artifact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum ArtifactStatus {
    #[default]
    Pending,
    Ready,
    Failed,
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
    /// The file's name relative to its root. For a record with [`Self::file`]
    /// set it is relative to [`Self::file_root`] (e.g. `"deck.pptx"`); for a
    /// legacy record it is relative to `<workspace>/artifacts/`
    /// (e.g. `"<uuid>/deck.pptx"`).
    pub path: String,
    /// Absolute path of the deliverable in the visible files folder (#5505).
    /// `None` for records written before that folder existed; those resolve
    /// under `<workspace>/artifacts/` until the boot migration moves them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// The files folder [`Self::file`] was placed in. Recorded per artifact so
    /// changing the folder later does not strand existing files, and so the
    /// escape guard has a root to check `file` against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_root: Option<String>,
    /// Artifact file size in bytes.
    pub size_bytes: u64,
    /// Current lifecycle status.
    pub status: ArtifactStatus,
    /// UTC timestamp when this artifact was created.
    pub created_at: DateTime<Utc>,
    /// Failure reason set when [`ArtifactStatus::Failed`]; `None`
    /// otherwise. Persisted so list/get RPCs can surface why a build
    /// did not produce a usable file without callers having to scrape
    /// stderr from a separate log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Chat thread that produced the artifact, captured from
    /// the producing turn's [`crate::security::approval::ApprovalChatContext`] at create-time
    /// (#3226). `None` for CLI / cron / sub-agent paths and for legacy
    /// `meta.json` files written before this field existed — same convention
    /// as the `thread_id` carried on the producer events
    /// (`DomainEvent::ArtifactReady` / `Failed`). Used by
    /// `ai_list_artifacts(thread_id = …)` to rebuild `ChatFilesPanel` from
    /// disk after a redux-persist purge / fresh-device boot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Provider-assigned tool-call id of the producing tool invocation,
    /// captured at [`super::store::create_artifact`] time and carried on
    /// every lifecycle event (`ArtifactPending`/`Ready`/`Failed`) so the UI
    /// can correlate the card with the tool-call bubble that produced it.
    /// `None` for producers that ran outside a harness tool-call context
    /// (CLI, cron) and for `meta.json` files written before this field
    /// existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
