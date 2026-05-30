//! Core types for the `task_sources` domain.
//!
//! A [`TaskSource`] is a user-configured pull of work items from an
//! external tool (GitHub, Notion, Linear, ClickUp) with a per-provider
//! [`FilterSpec`]. The periodic poll fetches matching items, normalizes
//! them ([`super::NormalizedTask`]), enriches them ([`EnrichedTask`]),
//! and routes them onto the agent's todo board.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// External tool a [`TaskSource`] pulls from. The string form matches
/// the Composio toolkit slug, so it keys directly into the provider
/// registry (`get_provider`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSlug {
    Github,
    Notion,
    Linear,
    Clickup,
}

impl ProviderSlug {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Notion => "notion",
            Self::Linear => "linear",
            Self::Clickup => "clickup",
        }
    }

    /// Parse a toolkit slug into a `ProviderSlug`. Case-insensitive.
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "github" => Ok(Self::Github),
            "notion" => Ok(Self::Notion),
            "linear" => Ok(Self::Linear),
            "clickup" => Ok(Self::Clickup),
            other => Err(format!(
                "unknown task source provider '{other}' (expected github|notion|linear|clickup)"
            )),
        }
    }
}

/// Per-provider, user-configured filter. Tagged by `provider` on the
/// wire so the frontend can render typed pickers; each variant carries a
/// free-form `extra` object as an advanced escape hatch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum FilterSpec {
    Github {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repo: Option<String>,
        #[serde(default)]
        labels: Vec<String>,
        #[serde(default)]
        assignee_is_me: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<String>,
        #[serde(default)]
        extra: Value,
    },
    Notion {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database_id: Option<String>,
        #[serde(default)]
        assigned_to_me: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        #[serde(default)]
        extra: Value,
    },
    Linear {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        team_id: Option<String>,
        #[serde(default)]
        assignee_is_me: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<String>,
        #[serde(default)]
        extra: Value,
    },
    Clickup {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        team_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        list_id: Option<String>,
        #[serde(default)]
        assignee_is_me: bool,
        #[serde(default)]
        extra: Value,
    },
}

impl FilterSpec {
    /// The provider this filter targets — must match the owning
    /// [`TaskSource::provider`].
    pub fn provider(&self) -> ProviderSlug {
        match self {
            Self::Github { .. } => ProviderSlug::Github,
            Self::Notion { .. } => ProviderSlug::Notion,
            Self::Linear { .. } => ProviderSlug::Linear,
            Self::Clickup { .. } => ProviderSlug::Clickup,
        }
    }
}

/// How enriched tasks are routed once fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceTarget {
    /// Append a todo card AND dispatch a triage turn so an agent may
    /// start working immediately (triage still gates noise).
    AgentTodoProactive,
    /// Append a todo card only; never auto-start an agent turn.
    TodoOnly,
}

impl Default for SourceTarget {
    fn default() -> Self {
        Self::AgentTodoProactive
    }
}

/// Why a fetch ran — mirrors the provider `SyncReason` semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchReason {
    /// First fetch right after an OAuth connection is created.
    ConnectionCreated,
    /// Periodic background poll.
    Periodic,
    /// Explicit user / RPC trigger.
    Manual,
}

impl FetchReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConnectionCreated => "connection_created",
            Self::Periodic => "periodic",
            Self::Manual => "manual",
        }
    }
}

/// A persisted task source configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSource {
    pub id: String,
    pub provider: ProviderSlug,
    /// Composio connection id; `None` resolves the connection by toolkit
    /// at fetch time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub enabled: bool,
    pub filter: FilterSpec,
    pub interval_secs: u64,
    pub target: SourceTarget,
    pub max_tasks_per_fetch: u32,
    /// Static executor routing (G7): a personality / skill / agent handle that
    /// every card from this source is pre-assigned to, so the dispatcher runs
    /// it deterministically without the LLM router. `None` leaves cards
    /// unassigned (router / poller decides).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_executor: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fetch_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_status: Option<String>,
}

/// Partial update payload for [`super::store::update_source`]. Each
/// `Some` field is applied; `None` leaves the existing value untouched.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSourcePatch {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub filter: Option<FilterSpec>,
    #[serde(default)]
    pub interval_secs: Option<u64>,
    #[serde(default)]
    pub target: Option<SourceTarget>,
    #[serde(default)]
    pub max_tasks_per_fetch: Option<u32>,
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub assigned_executor: Option<String>,
}

/// An enriched, agent-ready task produced by [`super::enrich`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrichedTask {
    pub task: super::NormalizedTask,
    /// One- to two-line LLM summary (falls back to the title).
    pub summary: String,
    /// Urgency score in `0.0..=1.0`.
    pub urgency: f32,
    #[serde(default)]
    pub linked_people: Vec<String>,
    #[serde(default)]
    pub linked_memory_ids: Vec<String>,
    /// Actionable prompt handed to the agent turn.
    pub agent_prompt: String,
    pub enriched_at: DateTime<Utc>,
}

/// Result of a single fetch pass over one source.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchOutcome {
    pub source_id: String,
    pub provider: String,
    /// Tasks returned by the provider.
    pub fetched: usize,
    /// Tasks newly routed (enriched + carded) this pass.
    pub routed: usize,
    /// Tasks skipped because they were already ingested.
    pub skipped_dupe: usize,
    /// Optional human-readable status line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn provider_slug_roundtrips() {
        for p in [
            ProviderSlug::Github,
            ProviderSlug::Notion,
            ProviderSlug::Linear,
            ProviderSlug::Clickup,
        ] {
            assert_eq!(ProviderSlug::parse(p.as_str()).unwrap(), p);
        }
        assert_eq!(ProviderSlug::parse("GitHub").unwrap(), ProviderSlug::Github);
        assert!(ProviderSlug::parse("jira").is_err());
    }

    #[test]
    fn provider_slug_serde_is_snake_case() {
        assert_eq!(
            serde_json::to_string(&ProviderSlug::Clickup).unwrap(),
            "\"clickup\""
        );
    }

    #[test]
    fn filter_spec_tagged_by_provider() {
        let f = FilterSpec::Github {
            repo: Some("owner/name".into()),
            labels: vec!["bug".into()],
            assignee_is_me: true,
            state: Some("open".into()),
            extra: json!({}),
        };
        let s = serde_json::to_value(&f).unwrap();
        assert_eq!(s["provider"], "github");
        assert_eq!(s["repo"], "owner/name");
        assert_eq!(f.provider(), ProviderSlug::Github);

        let back: FilterSpec = serde_json::from_value(s).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn notion_filter_roundtrips_with_board() {
        let f = FilterSpec::Notion {
            database_id: Some("db-1".into()),
            assigned_to_me: true,
            status: Some("In Progress".into()),
            extra: json!({"page_size": 10}),
        };
        let back: FilterSpec = serde_json::from_value(serde_json::to_value(&f).unwrap()).unwrap();
        assert_eq!(back, f);
        assert_eq!(back.provider(), ProviderSlug::Notion);
    }

    #[test]
    fn source_target_defaults_to_proactive() {
        assert_eq!(SourceTarget::default(), SourceTarget::AgentTodoProactive);
    }

    #[test]
    fn fetch_reason_as_str() {
        assert_eq!(FetchReason::Periodic.as_str(), "periodic");
        assert_eq!(
            FetchReason::ConnectionCreated.as_str(),
            "connection_created"
        );
        assert_eq!(FetchReason::Manual.as_str(), "manual");
    }

    #[test]
    fn task_source_serializes_camel_case() {
        let src = TaskSource {
            id: "s1".into(),
            provider: ProviderSlug::Linear,
            connection_id: None,
            name: Some("My Linear".into()),
            enabled: true,
            filter: FilterSpec::Linear {
                team_id: Some("team-1".into()),
                assignee_is_me: true,
                state: None,
                extra: json!({}),
            },
            interval_secs: 1800,
            target: SourceTarget::TodoOnly,
            max_tasks_per_fetch: 25,
            assigned_executor: None,
            created_at: DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            last_fetch_at: None,
            last_status: None,
        };
        let v = serde_json::to_value(&src).unwrap();
        assert_eq!(v["maxTasksPerFetch"], 25);
        assert_eq!(v["intervalSecs"], 1800);
        assert_eq!(v["target"], "todo_only");
        // connection_id / last_fetch_at omitted when None.
        assert!(v.get("connectionId").is_none());
        let back: TaskSource = serde_json::from_value(v).unwrap();
        assert_eq!(back, src);
    }
}
