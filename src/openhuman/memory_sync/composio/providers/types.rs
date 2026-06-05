//! Shared types for Composio provider implementations.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use crate::openhuman::composio::client::{
    create_composio_client, direct_execute, ComposioClient, ComposioClientKind,
};
use crate::openhuman::composio::types::ComposioExecuteResponse;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;

/// Reason a sync was triggered. Providers can use this to decide
/// whether to do a full backfill or an incremental pull.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncReason {
    /// First sync immediately after an OAuth handoff completes.
    ConnectionCreated,
    /// Periodic background sync from the scheduler.
    Periodic,
    /// Explicit user-driven sync from RPC / UI.
    Manual,
}

impl SyncReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            SyncReason::ConnectionCreated => "connection_created",
            SyncReason::Periodic => "periodic",
            SyncReason::Manual => "manual",
        }
    }
}

/// What kind of work an ingested task implies. GitHub's issues-and-PRs
/// search returns both shapes, and the job differs fundamentally —
/// *resolve* an issue vs *review* a pull request — so providers tag each
/// task and the `task_sources` enrichment phrases the objective / agent
/// prompt accordingly (the triage LLM then knows what to do). Providers
/// that don't distinguish (notion, linear, clickup) leave this `Generic`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// No issue/PR distinction — the default for non-code providers.
    #[default]
    Generic,
    /// A tracker issue: the job is to resolve / implement it.
    Issue,
    /// A pull request: the job is to review it (read the diff, give feedback).
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

/// Normalized user profile shape returned by every provider.
///
/// The shared fields (`display_name`, `email`, `username`, `avatar_url`,
/// `profile_url`)
/// cover what the desktop UI actually needs to render a connected
/// account card. Anything provider-specific (Gmail's `messagesTotal`,
/// Notion's workspace ids, …) goes into [`extras`](Self::extras) so
/// callers don't have to widen the shape every time a new toolkit
/// lands.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderUserProfile {
    pub toolkit: String,
    pub connection_id: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub profile_url: Option<String>,
    /// Provider-specific extras (raw JSON object).
    #[serde(default)]
    pub extras: serde_json::Value,
}

/// Result of a provider sync run. Mostly used for logging + UI status.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncOutcome {
    pub toolkit: String,
    pub connection_id: Option<String>,
    pub reason: String,
    pub items_ingested: usize,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    pub summary: String,
    /// Provider-specific extras (raw JSON object).
    #[serde(default)]
    pub details: serde_json::Value,
}

impl SyncOutcome {
    pub fn elapsed_ms(&self) -> u64 {
        self.finished_at_ms.saturating_sub(self.started_at_ms)
    }
}

/// A provider-agnostic, structured work item produced by
/// [`super::ComposioProvider::fetch_tasks`].
///
/// Unlike the `sync()` path — which persists upstream items into the
/// memory store as passive context — `fetch_tasks` *returns* normalized
/// tasks so the `task_sources` domain can enrich them and route them
/// onto the agent's todo board. Every native task provider (github,
/// notion, linear, clickup) maps its upstream payload shape into this
/// common envelope.
///
/// `source_id` is left empty by providers and stamped by the
/// `task_sources` pipeline with the originating `TaskSource.id` — a
/// provider has no knowledge of which configured source asked for the
/// fetch.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedTask {
    /// Upstream provider's stable id for the item (issue/task/page id).
    pub external_id: String,
    /// The `TaskSource.id` that produced this task. Empty until the
    /// pipeline stamps it.
    #[serde(default)]
    pub source_id: String,
    /// Toolkit slug, e.g. `"github"`.
    pub provider: String,
    /// Whether this task is an issue, a pull request, or undifferentiated.
    /// Drives intent-aware objective / prompt phrasing in enrichment.
    #[serde(default)]
    pub kind: TaskKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// Due date as an ISO-8601 string, when the provider exposes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// Last-updated ISO-8601 timestamp — used for cursor advancement and
    /// edit-aware dedup (`{external_id}@{updated_at}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// The raw upstream payload, retained for enrichment / debugging.
    #[serde(default)]
    pub raw: serde_json::Value,
}

/// A selectable upstream task container (board / database / list) used to
/// populate a picker so the user chooses from a list instead of pasting a
/// raw id. Today this is a Notion database, later a Linear team or ClickUp
/// list. Surfaced to the task-source UI as `{ id, title }`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskContainer {
    /// Provider-native id (e.g. a Notion database id) used as the filter id.
    pub id: String,
    /// Human-readable label for the picker.
    pub title: String,
}

/// Provider-agnostic filter passed into
/// [`super::ComposioProvider::fetch_tasks`].
///
/// The `task_sources` domain builds this from a user-configured,
/// per-provider `FilterSpec`. Each provider reads only the fields that
/// apply to it (github reads `repo`/`labels`; notion reads
/// `database_id`; linear/clickup read `team_id`; …) and ignores the
/// rest. `extra` is a free-form escape hatch surfaced in the UI for
/// advanced provider-native query fragments.
/// How the GitHub task-source fetch reaches GitHub. Shipped desktop users
/// connect GitHub via Composio OAuth (no `gh` on PATH, no `GITHUB_TOKEN`),
/// while local dev / self-host setups often have the reverse. `Auto` does the
/// right thing for both; `Composio` / `Local` force a path when the user wants.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GithubFetchMode {
    /// Try the connected Composio account first; fall back to local `gh`/REST
    /// only when Composio is unavailable. The safe default — no regression for
    /// shipped users, still a true fallback for local/dev.
    #[default]
    Auto,
    /// Force the connected Composio account (classic shipped-app behaviour).
    Composio,
    /// Force local `gh` CLI / REST with a `GH_TOKEN`/`GITHUB_TOKEN` env token.
    Local,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskFetchFilter {
    /// Scope to items assigned to (or involving) the authenticated user.
    #[serde(default)]
    pub assignee_is_me: bool,
    /// GitHub fetch path selector (Composio vs local `gh`/REST). Default `Auto`.
    #[serde(default)]
    pub github_fetch_mode: GithubFetchMode,
    /// GitHub `owner/name` repository scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// GitHub label filter.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Issue/task state filter (e.g. `"open"`, `"todo"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Notion database (board) id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_id: Option<String>,
    /// Notion status property filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Linear / ClickUp team (workspace) id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// ClickUp list id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_id: Option<String>,
    /// Free-form provider-native filter fragment (advanced).
    #[serde(default)]
    pub extra: serde_json::Value,
    /// Hard cap on how many tasks a single fetch returns.
    #[serde(default)]
    pub max: u32,
}

impl TaskFetchFilter {
    /// Effective per-fetch item cap, defaulting to a safe bound when the
    /// caller leaves `max` unset (0).
    pub fn effective_max(&self) -> usize {
        if self.max == 0 {
            25
        } else {
            self.max as usize
        }
    }
}

/// Per-call context handed to provider methods.
///
/// `connection_id` is `None` when a method runs in a "no specific
/// connection" mode (e.g. an across-the-board periodic sync that
/// already iterated). For per-connection paths it is always populated.
///
/// **Mode-aware dispatch (#1710)**: pre-fix, `ProviderContext` cached a
/// pre-baked [`ComposioClient`] built once at construction time. Toggling
/// `composio.mode = "direct"` mid-session left provider syncs still
/// routing through the backend tinyhumans tenant. The current shape
/// keeps an [`Arc<Config>`] and resolves the underlying client per call
/// through [`ProviderContext::execute`], mirroring the agent-tool
/// migration in [`crate::openhuman::composio::tools::ComposioExecuteTool`].
/// Per-sync accumulator for Composio billable-action usage.
///
/// Lives behind a shared handle on [`ProviderContext`] so the single
/// `execute` chokepoint can tally every action a provider fires during one
/// sync run, regardless of which provider (gmail / slack / github / notion /
/// linear / clickup) or how many pages it paginates.
/// [`crate::openhuman::memory_sync::composio::run_connection_sync`] returns
/// the final tally alongside the [`SyncOutcome`] for the sync audit log
/// (#3111).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComposioUsage {
    /// Count of `execute` calls that returned a response this run.
    pub actions_called: u32,
    /// Sum of each response's backend-reported `cost_usd`.
    pub cost_usd: f64,
}

/// Shared, interior-mutable handle to a [`ComposioUsage`] tally. Cloning a
/// [`ProviderContext`] shares the same underlying counter, so the count is
/// stable no matter how the context is passed around within a sync.
pub type ComposioUsageHandle = Arc<Mutex<ComposioUsage>>;

#[derive(Clone)]
pub struct ProviderContext {
    pub config: Arc<Config>,
    pub toolkit: String,
    pub connection_id: Option<String>,
    /// Accumulates Composio billable-action usage across this context's
    /// lifetime. Defaulted at every construction site; only the sync path
    /// (`run_connection_sync`) reads it back. Non-sync callers (agent tools,
    /// task-source fetches) leave it at zero — harmless.
    pub usage: ComposioUsageHandle,
    /// Maximum items to fetch in a single sync pass.
    ///
    /// Set from the corresponding `MemorySourceEntry.max_items` field at
    /// sync-dispatch time. `None` means no cap beyond the provider's own
    /// internal upper bounds.
    pub max_items: Option<u32>,
    /// Maximum sync depth window in days.
    ///
    /// Set from `MemorySourceEntry.sync_depth_days`. When `Some(n)`, the
    /// provider only fetches items from the last `n` days. `None` means
    /// no additional depth restriction beyond the provider's cursor.
    pub sync_depth_days: Option<u32>,
}

impl ProviderContext {
    /// Build a context from the current config + a toolkit slug.
    ///
    /// Returns `None` only when we want to short-circuit early on the
    /// "user clearly not signed in" path. In the post-#1710 shape this
    /// is determined by attempting a factory resolve via
    /// [`create_composio_client`] and treating any error there as
    /// "skip silently" — the same UX as the pre-fix
    /// `build_composio_client(...).is_some()` probe, but routed
    /// through the mode-aware factory so direct-mode users (no backend
    /// session token, BYO key in keychain) aren't falsely treated as
    /// signed-out.
    pub fn from_config(
        config: Arc<Config>,
        toolkit: impl Into<String>,
        connection_id: Option<String>,
    ) -> Option<Self> {
        // Probe the factory: any successful resolve (Backend OR Direct)
        // means the user has *some* viable Composio client. Direct-mode
        // users typically have no backend session token, which would
        // make a `build_composio_client` probe return None and falsely
        // skip them.
        match create_composio_client(&config) {
            Ok(_) => Some(Self {
                config,
                toolkit: toolkit.into(),
                connection_id,
                usage: ComposioUsageHandle::default(),
                max_items: None,
                sync_depth_days: None,
            }),
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "[composio:provider_context] from_config: factory probe failed; \
                     treating as not-signed-in"
                );
                None
            }
        }
    }

    /// Resolve the underlying composio client via the mode-aware
    /// factory and dispatch a single action. This is the canonical
    /// way for provider implementations to execute a Composio action
    /// — going through here ensures the live `composio.mode` toggle is
    /// honoured on every call (#1710).
    ///
    /// Returns the same [`ComposioExecuteResponse`] shape that
    /// [`ComposioClient::execute_tool`] used to return so existing
    /// provider call-sites can swap `ctx.client.execute_tool(...)` for
    /// `ctx.execute(...)` with no other changes.
    pub async fn execute(
        &self,
        action: &str,
        arguments: Option<serde_json::Value>,
    ) -> anyhow::Result<ComposioExecuteResponse> {
        // [#1710 Wave 4] Reload config fresh per execute so a mid-session
        // `composio.mode` toggle takes effect at the very next call. The
        // Arc<Config> snapshot held by `self` was taken at agent-init time
        // and is otherwise stale relative to subsequent set_api_key /
        // clear_api_key RPCs.
        //
        // Use `reload_config_snapshot_with_timeout` (anchored to the snapshot's
        // `config_path`) rather than `load_config_with_timeout` (which
        // re-resolves `OPENHUMAN_WORKSPACE` from the process env). The config
        // path is stable for the lifetime of a `ProviderContext` — it is set
        // at context creation from the agent's scoped config — so reading from
        // it always reaches the correct user workspace and avoids a data-race
        // in tests that share the process env.
        let live_config = config_rpc::reload_config_snapshot_with_timeout(&self.config)
            .await
            .map_err(|e| {
                tracing::warn!(
                    action = %action,
                    toolkit = %self.toolkit,
                    error = %e,
                    "[composio:provider_context] execute: reload_config failed"
                );
                anyhow::anyhow!("composio provider_context: failed to reload live config: {e}")
            })?;
        let kind = create_composio_client(&live_config)?;
        let result = match kind {
            ComposioClientKind::Backend(client) => {
                tracing::debug!(
                    action = %action,
                    toolkit = %self.toolkit,
                    "[composio:provider_context] execute: backend variant"
                );
                client.execute_tool(action, arguments).await
            }
            ComposioClientKind::Direct(direct) => {
                tracing::debug!(
                    action = %action,
                    toolkit = %self.toolkit,
                    "[composio:provider_context] execute: direct variant"
                );
                direct_execute(&direct, action, arguments, &live_config.composio.entity_id).await
            }
        };

        // Tally billable-action usage at the single chokepoint every provider
        // routes through (#3111). We count any *completed* round-trip — even a
        // provider-reported failure (`successful == false`) is a billable call
        // — and sum the backend-reported `cost_usd`. Transport errors (the
        // `Err` arm) never reached Composio, so they don't count. The lock is
        // held only for the increment, never across an `.await`.
        if let Ok(ref resp) = result {
            if let Ok(mut usage) = self.usage.lock() {
                usage.actions_called = usage.actions_called.saturating_add(1);
                usage.cost_usd += resp.cost_usd;
            }
        }
        result
    }

    /// Resolve a `ComposioClient` for callers that need a handle to
    /// pass to helpers built around the old `&ComposioClient` API
    /// (e.g. `slack::users::SlackUsers::fetch`,
    /// `slack::provider::execute_with_retry`).
    ///
    /// Returns `Err` when the live config selects direct mode — these
    /// legacy helpers were written against the backend-tenant
    /// `ComposioClient` and have not yet been ported to the factory.
    /// Direct-mode users hit this path as a hard error rather than
    /// silently routing through the wrong tenant.
    pub async fn backend_client(&self) -> anyhow::Result<ComposioClient> {
        // [#1710 Wave 4] Reload config fresh per call so a mid-session
        // `composio.mode` toggle takes effect immediately. The Arc<Config>
        // snapshot held by `self` was taken at agent-init time and is
        // otherwise stale relative to subsequent set_api_key /
        // clear_api_key RPCs.
        //
        // Anchored to the snapshot's config_path (not OPENHUMAN_WORKSPACE)
        // for the same isolation reason as `execute`.
        let live_config = config_rpc::reload_config_snapshot_with_timeout(&self.config)
            .await
            .map_err(|e| {
                tracing::warn!(
                    toolkit = %self.toolkit,
                    error = %e,
                    "[composio:provider_context] backend_client: reload_config failed"
                );
                anyhow::anyhow!(
                    "composio provider_context.backend_client: failed to reload live config: {e}"
                )
            })?;
        match create_composio_client(&live_config)? {
            ComposioClientKind::Backend(client) => Ok(client),
            ComposioClientKind::Direct(_) => Err(anyhow::anyhow!(
                "composio direct mode is not yet supported on this provider's helper path; \
                 toolkit={}",
                self.toolkit
            )),
        }
    }

    /// Memory client handle if the global memory singleton is ready.
    /// Used by providers that want to persist sync snapshots.
    pub fn memory_client(&self) -> Option<crate::openhuman::memory_store::MemoryClientRef> {
        #[cfg(test)]
        {
            return crate::openhuman::memory_store::MemoryClient::from_workspace_dir(
                self.config.workspace_dir.clone(),
            )
            .ok()
            .map(std::sync::Arc::new);
        }

        #[cfg(not(test))]
        crate::openhuman::memory::global::client_if_ready()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole #3111 tally relies on the `usage` handle being *shared*
    /// across `ProviderContext` clones: a provider's `sync` runs against a
    /// clone (or the same ctx passed by `&`), accumulates via `execute`, and
    /// `run_connection_sync` reads the count back from its own handle. Pin
    /// that the `Arc<Mutex<_>>` is genuinely shared so a clone's increments
    /// are visible from the original — if this regressed to a per-clone
    /// counter, the audit cost would silently always read zero.
    #[test]
    fn usage_handle_is_shared_across_context_clones() {
        let ctx = ProviderContext {
            config: Arc::new(Config::default()),
            toolkit: "gmail".to_string(),
            connection_id: None,
            usage: ComposioUsageHandle::default(),
            max_items: None,
            sync_depth_days: None,
        };
        let cloned = ctx.clone();

        // Simulate two `execute` round-trips accumulating on the clone.
        {
            let mut usage = cloned.usage.lock().expect("lock usage");
            usage.actions_called = usage.actions_called.saturating_add(2);
            usage.cost_usd += 0.015;
        }

        // The original handle must observe the clone's tally.
        let observed = ctx.usage.lock().expect("lock usage");
        assert_eq!(observed.actions_called, 2);
        assert!((observed.cost_usd - 0.015).abs() < 1e-9);
    }

    /// `ComposioUsage` defaults to a zero tally — the value
    /// `run_connection_sync` returns for a sync that fired no Composio
    /// actions, and what non-sync `ProviderContext` callers carry.
    #[test]
    fn composio_usage_defaults_to_zero() {
        let usage = ComposioUsage::default();
        assert_eq!(usage.actions_called, 0);
        assert_eq!(usage.cost_usd, 0.0);
    }

    // `ProviderContext::execute` and `ProviderContext::backend_client` reload
    // config from `ctx.config.config_path` (via `reload_config_snapshot_with_timeout`)
    // rather than from the process-global `OPENHUMAN_WORKSPACE`. Tests
    // therefore only need to persist the config to `config_path` — no env var
    // manipulation required.

    #[tokio::test]
    async fn provider_context_execute_resolves_via_factory_at_call_time() {
        // Build a context against a direct-mode config (no backend
        // session token, only the inline direct api_key). The factory
        // must pick the `Direct` variant on `execute` — pre-fix the
        // `client: ComposioClient` field was always backend, so this
        // path would have surfaced a backend session lookup error
        // even with `mode = "direct"`.
        let tmp = tempfile::tempdir().expect("tempdir");

        let mut config = Config::default();
        config.config_path = tmp.path().join("config.toml");
        config.workspace_dir = tmp.path().join("workspace");
        config.secrets.encrypt = false;
        config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
        config.composio.api_key = Some("test-direct-key".to_string());
        config.save().await.expect("save fake config to disk");

        let ctx = ProviderContext {
            config: Arc::new(config),
            toolkit: "gmail".to_string(),
            connection_id: None,
            usage: ComposioUsageHandle::default(),
            max_items: None,
            sync_depth_days: None,
        };
        let res = ctx.execute("GMAIL_FETCH_EMAILS", None).await;
        // The actual HTTP call will fail in the unit-test sandbox, but
        // the error must come from the direct path — never a backend
        // session lookup, which is the smoking gun for the pre-fix bug.
        if let Err(e) = res {
            let msg = e.to_string();
            assert!(
                !msg.contains("no backend session"),
                "direct-mode execute must not surface backend session artifacts: {msg}"
            );
        }
    }

    #[tokio::test]
    async fn provider_context_execute_backend_branch_without_session_errors_cleanly() {
        // Default `Config` (mode = "backend") with no stored session
        // token: the factory should return a backend-session error from
        // `ctx.execute`. Verifies the backend branch is reachable and
        // the error surface is sensible.
        let tmp = tempfile::tempdir().expect("tempdir");

        let mut config = Config::default();
        config.config_path = tmp.path().join("config.toml");
        config.workspace_dir = tmp.path().join("workspace");
        config.secrets.encrypt = false;
        config.save().await.expect("save fake config to disk");

        let ctx = ProviderContext {
            config: Arc::new(config),
            toolkit: "gmail".to_string(),
            connection_id: None,
            usage: ComposioUsageHandle::default(),
            max_items: None,
            sync_depth_days: None,
        };
        let res = ctx.execute("GMAIL_FETCH_EMAILS", None).await;
        let err = res.expect_err("no backend session must error");
        let msg = err.to_string();
        assert!(
            msg.contains("backend") || msg.contains("session"),
            "expected backend-session error, got: {msg}"
        );
    }
}
