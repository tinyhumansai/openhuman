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

    /// Whether this provider has a working task-fetch path today.
    ///
    /// **This is the single place that answers the question**, so the
    /// periodic scheduler and the RPC surface cannot drift apart on it.
    ///
    /// Every provider returns `false` right now: the pipeline's fetch step
    /// (`pipeline::fetch_tasks_unavailable`) is an unconditional `Err` for
    /// every toolkit, because tinymemory v1.13.4 deleted
    /// `ComposioProvider::fetch_tasks` with no replacement. Nothing
    /// downstream of the fetch is broken — dedup, enrichment, routing and
    /// reconciliation all still work — so the moment a provider grows a real
    /// fetch again this flips to `true` for that variant and the scheduler
    /// starts polling it, with no other change.
    ///
    /// It is deliberately a per-variant `match` rather than a blanket
    /// `false`: the restoration lands one toolkit at a time, and a
    /// wholesale constant would have to be redesigned on the first one
    /// instead of edited.
    pub fn can_fetch(self) -> bool {
        match self {
            Self::Github | Self::Notion | Self::Linear | Self::Clickup => false,
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
        /// How to fetch: Composio connection, local `gh`/REST, or `auto`
        /// (Composio-first with local fallback). Defaults to `auto`.
        #[serde(default)]
        fetch_mode: crate::openhuman::integrations::composio::providers::GithubFetchMode,
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
#[derive(Default)]
pub enum SourceTarget {
    /// Append a todo card AND dispatch a triage turn so an agent may
    /// start working immediately (triage still gates noise).
    #[default]
    AgentTodoProactive,
    /// Append a todo card only; never auto-start an agent turn.
    TodoOnly,
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
    /// Intent-framed goal for the card (`"Review pull request: …"` /
    /// `"Resolve issue: …"`), or the bare title for undifferentiated tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
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
    /// Previously ingested tasks removed because the upstream source no longer
    /// returns them for this filter/status.
    #[serde(default)]
    pub pruned: usize,
    /// Optional human-readable status line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
