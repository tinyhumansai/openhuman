//! The provider-agnostic work-item envelope: what a task fetch asks for, and
//! what it hands back.
//!
//! This is the second of the two things a Composio provider does. `sync`
//! persists upstream items into the memory store as passive context;
//! `fetch_tasks` *returns* [`NormalizedTask`]s so the host can enrich them and
//! route them onto the agent's todo board. Every native task provider (GitHub,
//! Notion, Linear, ClickUp) maps its upstream payload into this one envelope,
//! which is why the envelope — and not the payloads — is what crosses the bus.
//!
//! # A note on the wire casing
//!
//! [`NormalizedTask`], [`TaskContainer`] and [`TaskFetchFilter`] serialise
//! `camelCase`; the enums serialise `snake_case`. That is not an oversight: the
//! structs are read by the task-source UI, which is TypeScript, while the enum
//! tags are also written into a card's `source_metadata` and compared as
//! strings on the Rust side. Both forms are persisted — treat every field name
//! and every variant name as a compatibility surface.

use serde::{Deserialize, Serialize};

/// What kind of work an ingested task implies.
///
/// GitHub's issues-and-pull-requests search returns both shapes and the job
/// differs fundamentally — *resolve* an issue versus *review* a pull request —
/// so providers tag each task and the enrichment phrases the objective and the
/// agent prompt accordingly. Providers that do not distinguish (Notion, Linear,
/// ClickUp) leave this [`Generic`](Self::Generic).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// No issue/pull-request distinction — the default for non-code providers.
    #[default]
    Generic,
    /// A tracker issue: the job is to resolve or implement it.
    Issue,
    /// A pull request: the job is to review it — read the diff, give feedback.
    PullRequest,
}

impl TaskKind {
    /// Stable lowercase tag, mirrored into the card's `source_metadata`.
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskKind::Generic => "generic",
            TaskKind::Issue => "issue",
            TaskKind::PullRequest => "pull_request",
        }
    }
}

/// How the GitHub task-source fetch reaches GitHub.
///
/// Shipped desktop users connect GitHub through Composio OAuth — no `gh` on
/// `PATH`, no `GITHUB_TOKEN` — while local development and self-hosted setups
/// often have the reverse. [`Auto`](Self::Auto) does the right thing for both;
/// the other two force a path when the user wants one.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GithubFetchMode {
    /// Try the connected Composio account first; fall back to local `gh` or
    /// REST only when Composio is unavailable.
    ///
    /// The safe default: no regression for shipped users, still a true fallback
    /// for local and development setups.
    #[default]
    Auto,
    /// Force the connected Composio account — the classic shipped-app path.
    Composio,
    /// Force the local `gh` CLI or REST with a `GH_TOKEN` / `GITHUB_TOKEN`
    /// environment token.
    Local,
}

/// A provider-agnostic, structured work item returned by a task fetch.
///
/// `source_id` is left empty by providers and stamped by the host's task-source
/// pipeline with the originating source id — a provider has no knowledge of
/// which configured source asked for the fetch.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedTask {
    /// The upstream provider's stable id for the item — issue, task or page id.
    pub external_id: String,
    /// The task source that produced this task. Empty until the pipeline
    /// stamps it.
    #[serde(default)]
    pub source_id: String,
    /// Toolkit slug, e.g. `"github"`.
    pub provider: String,
    /// Whether this task is an issue, a pull request, or undifferentiated.
    ///
    /// Drives intent-aware objective and prompt phrasing during enrichment.
    #[serde(default)]
    pub kind: TaskKind,
    /// Human-readable title, as the provider spells it.
    pub title: String,
    /// Body text or description, when the provider returns one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Canonical web URL for the item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Provider-native status string, e.g. `"open"` or `"todo"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Whoever the item is assigned to upstream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// Due date as an ISO-8601 string, when the provider exposes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    /// Provider-native labels or tags.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Provider-native priority string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// Last-updated ISO-8601 timestamp — used for cursor advancement and
    /// edit-aware dedup (`{external_id}@{updated_at}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// The raw upstream payload, retained for enrichment and debugging.
    #[serde(default)]
    pub raw: serde_json::Value,
}

/// A selectable upstream task container — a board, database or list.
///
/// Populates a picker so the user chooses from a list instead of pasting a raw
/// id. Today this is a Notion database; later a Linear team or a ClickUp list.
/// Surfaced to the task-source UI as `{ id, title }`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskContainer {
    /// Provider-native id, e.g. a Notion database id, used as the filter id.
    pub id: String,
    /// Human-readable label for the picker.
    pub title: String,
}

/// Provider-agnostic filter passed into a task fetch.
///
/// The host builds this from a user-configured, per-provider filter spec. Each
/// provider reads only the fields that apply to it — GitHub reads `repo` and
/// `labels`, Notion reads `database_id`, Linear and ClickUp read `team_id` —
/// and ignores the rest. [`extra`](Self::extra) is a free-form escape hatch
/// surfaced in the UI for advanced provider-native query fragments.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskFetchFilter {
    /// Scope to items assigned to — or involving — the authenticated user.
    #[serde(default)]
    pub assignee_is_me: bool,
    /// GitHub fetch-path selector. Defaults to [`GithubFetchMode::Auto`].
    #[serde(default)]
    pub github_fetch_mode: GithubFetchMode,
    /// GitHub `owner/name` repository scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// GitHub label filter.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Issue or task state filter, e.g. `"open"` or `"todo"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Notion database — board — id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_id: Option<String>,
    /// Notion status property filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Linear or ClickUp team — workspace — id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// ClickUp list id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_id: Option<String>,
    /// Free-form provider-native filter fragment, for advanced users.
    #[serde(default)]
    pub extra: serde_json::Value,
    /// Hard cap on how many tasks a single fetch returns. `0` means "unset";
    /// see [`effective_max`](Self::effective_max).
    #[serde(default)]
    pub max: u32,
}

impl TaskFetchFilter {
    /// Effective per-fetch item cap.
    ///
    /// `max` is `#[serde(default)]`, so an unset filter arrives as `0`. Reading
    /// that literally would mean "fetch nothing", which is never what a caller
    /// who omitted the field wanted, so an unset cap becomes a safe bound of 25
    /// instead.
    pub fn effective_max(&self) -> usize {
        if self.max == 0 {
            25
        } else {
            self.max as usize
        }
    }
}

#[cfg(test)]
#[path = "tasks_tests.rs"]
mod tests;
