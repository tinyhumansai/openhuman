//! Shared types for Composio provider implementations.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
#[derive(Clone)]
pub struct ProviderContext {
    pub config: Arc<Config>,
    pub toolkit: String,
    pub connection_id: Option<String>,
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
        let live_config = config_rpc::load_config_with_timeout().await.map_err(|e| {
            tracing::warn!(
                action = %action,
                toolkit = %self.toolkit,
                error = %e,
                "[composio:provider_context] execute: load_config failed"
            );
            anyhow::anyhow!("composio provider_context: failed to load live config: {e}")
        })?;
        let kind = create_composio_client(&live_config)?;
        match kind {
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
        }
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
        let live_config = config_rpc::load_config_with_timeout().await.map_err(|e| {
            tracing::warn!(
                toolkit = %self.toolkit,
                error = %e,
                "[composio:provider_context] backend_client: load_config failed"
            );
            anyhow::anyhow!(
                "composio provider_context.backend_client: failed to load live config: {e}"
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
    pub fn memory_client(&self) -> Option<crate::openhuman::memory::MemoryClientRef> {
        crate::openhuman::memory::global::client_if_ready()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Both `ProviderContext::execute` and `ProviderContext::backend_client`
    // now reload config via `config_rpc::load_config_with_timeout()` per
    // call (#1710 Wave 4), so the injected `Arc<Config>` no longer drives
    // the factory — the live on-disk config under `OPENHUMAN_WORKSPACE`
    // does. Both tests below therefore set up an isolated, persisted
    // config under `TEST_ENV_LOCK` rather than relying on a constructed
    // `Arc<Config>` helper.

    #[tokio::test]
    async fn provider_context_execute_resolves_via_factory_at_call_time() {
        // Build a context against a direct-mode config (no backend
        // session token, only the inline direct api_key). The factory
        // must pick the `Direct` variant on `execute` — pre-fix the
        // `client: ComposioClient` field was always backend, so this
        // path would have surfaced a backend session lookup error
        // even with `mode = "direct"`.
        //
        // Production `ctx.execute(..)` calls `load_config_with_timeout()`
        // per call which reads from `~/.openhuman/config.toml` (or the
        // workspace pointed at by `OPENHUMAN_WORKSPACE`). To isolate
        // the test from the dev's real config we hold `TEST_ENV_LOCK`,
        // point `OPENHUMAN_WORKSPACE` at a tempdir, and persist the
        // test's `Config` to that tempdir's `config.toml` before
        // invoking `execute`. Without the lock this test also races the
        // shared `OPENHUMAN_WORKSPACE` env var against the other
        // `load_config_with_timeout`-driven composio tests.
        use crate::openhuman::config::TEST_ENV_LOCK;
        let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let tmp = tempfile::tempdir().expect("tempdir");
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }

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

        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }

    #[tokio::test]
    async fn provider_context_execute_backend_branch_without_session_errors_cleanly() {
        // Default `Config` (mode = "backend") with no stored session
        // token: the factory should return a backend-session error from
        // `ctx.execute`. Verifies the backend branch is reachable and
        // the error surface is sensible.
        //
        // Production `ctx.execute(..)` calls `load_config_with_timeout()`
        // per call which reads from `~/.openhuman/config.toml` (or the
        // workspace pointed at by `OPENHUMAN_WORKSPACE`). To isolate
        // the test from the dev's real config we hold `TEST_ENV_LOCK`,
        // point `OPENHUMAN_WORKSPACE` at a tempdir, and persist the
        // test's `Config` to that tempdir's `config.toml` before
        // invoking `execute`.
        use crate::openhuman::config::TEST_ENV_LOCK;
        let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let tmp = tempfile::tempdir().expect("tempdir");
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }

        let mut config = Config::default();
        config.config_path = tmp.path().join("config.toml");
        config.workspace_dir = tmp.path().join("workspace");
        config.secrets.encrypt = false;
        config.save().await.expect("save fake config to disk");

        let ctx = ProviderContext {
            config: Arc::new(config),
            toolkit: "gmail".to_string(),
            connection_id: None,
        };
        let res = ctx.execute("GMAIL_FETCH_EMAILS", None).await;
        let err = res.expect_err("no backend session must error");
        let msg = err.to_string();
        assert!(
            msg.contains("backend") || msg.contains("session"),
            "expected backend-session error, got: {msg}"
        );

        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }
}
