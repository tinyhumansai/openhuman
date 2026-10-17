//! Core types for the `task_sources` domain.
//!
//! A [`TaskSource`] is a user-configured pull of work items from an
//! external tool (GitHub, Notion, Linear, ClickUp) with a per-provider
//! [`FilterSpec`] (the provider, filter, target and reason types are the
//! connector contract's). The periodic poll fetches matching items, normalizes
//! them ([`super::NormalizedTask`]), enriches them ([`EnrichedTask`]),
//! and routes them onto the agent's todo board.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use tinyconnectors_bus::vocabulary::{FetchReason, FilterSpec, ProviderSlug, SourceTarget};

/// Whether `provider` has a working task-fetch path today.
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
/// instead of edited. The provider and filter types themselves are the
/// connector contract's (`tinyconnectors_bus::vocabulary`); whether the host
/// can poll one is the host's call.
#[must_use]
pub fn provider_can_fetch(provider: ProviderSlug) -> bool {
    match provider {
        ProviderSlug::Github
        | ProviderSlug::Notion
        | ProviderSlug::Linear
        | ProviderSlug::Clickup => false,
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
