//! Maps OpenHuman `MemoryEntry` / `MemoryCategory` to the agentmemory REST
//! wire shapes and back.
//!
//! Wire contract: <https://github.com/rohitg00/agentmemory> — see the
//! upstream README for the full endpoint list and field semantics.
//!
//! agentmemory has a richer wire shape (concepts, files, strength, version,
//! supersedes) that the backend leaves at defaults — those fields are
//! internal to agentmemory's lifecycle layer and don't need to round-trip
//! through OpenHuman's trait. We map the OpenHuman-visible columns and let
//! agentmemory own the rest.

use crate::openhuman::memory::traits::{MemoryCategory, MemoryEntry};
use serde::{Deserialize, Serialize};

/// Globally well-known "default" project name used when an OpenHuman caller
/// doesn't pass a namespace. Matches the trait's `GLOBAL_NAMESPACE` semantics.
pub const DEFAULT_PROJECT: &str = "default";

/// agentmemory's per-memory `type` field. `MemoryCategory::Core` maps to
/// "fact", everything `MemoryCategory::Daily` / `Conversation` maps to
/// "conversation", and `Custom(s)` maps to "fact" with `s` rolled into the
/// `concepts` array so it remains queryable.
fn category_to_type(category: &MemoryCategory) -> &'static str {
    match category {
        MemoryCategory::Core | MemoryCategory::Custom(_) => "fact",
        MemoryCategory::Daily | MemoryCategory::Conversation => "conversation",
    }
}

fn type_to_category(t: Option<&str>, concepts: &[String]) -> MemoryCategory {
    match t {
        Some("conversation") => MemoryCategory::Conversation,
        Some("fact") | None => {
            if let Some(first) = concepts.first() {
                MemoryCategory::Custom(first.clone())
            } else {
                MemoryCategory::Core
            }
        }
        Some(other) => MemoryCategory::Custom(other.to_string()),
    }
}

/// Outgoing payload for `POST /agentmemory/remember`.
///
/// Owned fields rather than borrowed slices so the value remains
/// `Send + 'static`-friendly when handed to an async runtime / event bus.
#[derive(Debug, Clone, Serialize)]
pub struct RememberRequest {
    pub project: String,
    pub title: String,
    pub content: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub concepts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "sessionIds")]
    pub session_ids: Option<Vec<String>>,
}

impl RememberRequest {
    pub fn build(
        namespace: &str,
        key: &str,
        content: &str,
        category: &MemoryCategory,
        session_id: Option<&str>,
    ) -> Self {
        let concepts = match category {
            MemoryCategory::Custom(s) => vec![s.clone()],
            _ => Vec::new(),
        };
        let project = if namespace.is_empty() {
            DEFAULT_PROJECT.to_string()
        } else {
            namespace.to_string()
        };
        Self {
            project,
            title: key.to_string(),
            content: content.to_string(),
            kind: category_to_type(category).to_string(),
            concepts,
            session_ids: session_id.map(|s| vec![s.to_string()]),
        }
    }
}

/// Outgoing payload for `POST /agentmemory/smart-search`.
#[derive(Debug, Clone, Serialize)]
pub struct SmartSearchRequest {
    pub query: String,
    pub limit: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// Outgoing payload for `POST /agentmemory/forget`.
#[derive(Debug, Clone, Serialize)]
pub struct ForgetRequest {
    pub id: String,
}

/// Generic agentmemory memory row. agentmemory carries more fields than this
/// — we only deserialise what OpenHuman's `MemoryEntry` needs, leaving the
/// rest in a flatten-bag if a future caller wants them.
#[derive(Debug, Clone, Deserialize)]
pub struct WireMemory {
    pub id: String,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub concepts: Vec<String>,
    #[serde(default, rename = "sessionIds")]
    pub session_ids: Vec<String>,
    #[serde(default, rename = "updatedAt")]
    pub updated_at: Option<String>,
    #[serde(default, rename = "createdAt")]
    pub created_at: Option<String>,
    /// Present on smart-search hits, absent on direct fetches.
    #[serde(default)]
    pub score: Option<f64>,
}

impl WireMemory {
    /// Project the wire row into an OpenHuman `MemoryEntry`. `key` falls
    /// back to the agentmemory `id` when no title is present — for raw
    /// observation rows that never went through `remember`.
    pub fn into_entry(self) -> MemoryEntry {
        let timestamp = self
            .updated_at
            .or(self.created_at)
            .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string());
        let category = type_to_category(self.kind.as_deref(), &self.concepts);
        let key = self.title.unwrap_or_else(|| self.id.clone());
        let session_id = self.session_ids.into_iter().next();
        MemoryEntry {
            id: self.id,
            key,
            content: self.content.unwrap_or_default(),
            namespace: self.project,
            category,
            timestamp,
            session_id,
            score: self.score,
        }
    }
}

/// `POST /agentmemory/smart-search` response envelope. The `mode` field is
/// either `"full"` or `"compact"` depending on the requested `format`; both
/// modes share the same `results` array shape.
#[derive(Debug, Clone, Deserialize)]
pub struct SmartSearchResponse {
    #[serde(default)]
    pub results: Vec<WireMemory>,
}

/// `GET /agentmemory/memories` response envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct MemoriesResponse {
    #[serde(default)]
    pub memories: Vec<WireMemory>,
}

/// `GET /agentmemory/health` response envelope. agentmemory returns a much
/// richer payload; we only need the `memories` count.
#[derive(Debug, Clone, Deserialize)]
pub struct HealthResponse {
    #[serde(default)]
    pub memories: Option<usize>,
}

/// `GET /agentmemory/projects` response envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct ProjectsResponse {
    #[serde(default)]
    pub projects: Vec<ProjectSummary>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectSummary {
    pub name: String,
    #[serde(default)]
    pub count: usize,
    #[serde(default, rename = "lastUpdated")]
    pub last_updated: Option<String>,
}

/// `POST /agentmemory/remember` returns the saved memory's id.
#[derive(Debug, Clone, Deserialize)]
pub struct RememberResponse {
    pub id: String,
}

/// `POST /agentmemory/forget` returns whether anything was removed.
#[derive(Debug, Clone, Deserialize)]
pub struct ForgetResponse {
    #[serde(default)]
    pub forgotten: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remember_request_maps_core_to_fact() {
        let req = RememberRequest::build(
            "demo",
            "auth-stack",
            "uses HMAC bearer tokens",
            &MemoryCategory::Core,
            None,
        );
        assert_eq!(req.kind, "fact");
        assert!(req.concepts.is_empty());
        assert_eq!(req.project, "demo");
        assert_eq!(req.title, "auth-stack");
        assert!(req.session_ids.is_none());
    }

    #[test]
    fn remember_request_maps_custom_into_concepts() {
        let req = RememberRequest::build(
            "demo",
            "k",
            "v",
            &MemoryCategory::Custom("ops".into()),
            Some("ses-1"),
        );
        assert_eq!(req.kind, "fact");
        assert_eq!(req.concepts, vec!["ops".to_string()]);
        assert_eq!(req.session_ids.as_deref(), Some(&["ses-1".to_string()][..]));
    }

    #[test]
    fn remember_request_falls_back_to_default_project_on_empty_namespace() {
        let req = RememberRequest::build("", "k", "v", &MemoryCategory::Core, None);
        assert_eq!(req.project, DEFAULT_PROJECT);
    }

    #[test]
    fn wire_memory_into_entry_preserves_score_on_search_hits() {
        let wire = WireMemory {
            id: "mem_1".into(),
            project: Some("demo".into()),
            title: Some("auth-stack".into()),
            content: Some("uses HMAC".into()),
            kind: Some("fact".into()),
            concepts: vec!["auth".into()],
            session_ids: vec!["ses-1".into()],
            updated_at: Some("2026-05-14T00:00:00Z".into()),
            created_at: None,
            score: Some(0.87),
        };
        let entry = wire.into_entry();
        assert_eq!(entry.id, "mem_1");
        assert_eq!(entry.key, "auth-stack");
        assert_eq!(entry.namespace.as_deref(), Some("demo"));
        assert_eq!(entry.session_id.as_deref(), Some("ses-1"));
        assert_eq!(entry.score, Some(0.87));
        assert_eq!(entry.category, MemoryCategory::Custom("auth".into()));
    }

    #[test]
    fn wire_memory_into_entry_falls_back_to_id_when_title_missing() {
        let wire = WireMemory {
            id: "mem_2".into(),
            project: None,
            title: None,
            content: None,
            kind: Some("conversation".into()),
            concepts: vec![],
            session_ids: vec![],
            updated_at: None,
            created_at: Some("2026-05-14T00:00:00Z".into()),
            score: None,
        };
        let entry = wire.into_entry();
        assert_eq!(entry.key, "mem_2");
        assert_eq!(entry.category, MemoryCategory::Conversation);
        assert_eq!(entry.timestamp, "2026-05-14T00:00:00Z");
    }
}
