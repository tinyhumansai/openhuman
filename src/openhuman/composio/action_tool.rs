//! Per-action Composio tool wrapper.
//!
//! A [`ComposioActionTool`] is a [`Tool`] that represents exactly one
//! Composio action (e.g. `GMAIL_SEND_EMAIL`). It holds the action's
//! name, description, and parameter JSON schema so the LLM's native
//! tool-calling path can validate arguments before they hit the wire.
//!
//! These are constructed **dynamically at spawn time** by the sub-agent
//! runner when `integrations_agent` is spawned with a `toolkit` argument —
//! one tool per action in the chosen toolkit. The generic
//! [`ComposioExecuteTool`](super::tools::ComposioExecuteTool) dispatcher
//! is deliberately excluded from `integrations_agent`'s tool list in that
//! path so the model doesn't see two ways to call the same action.
//!
//! Lifetime: these tools live for the duration of a single sub-agent
//! spawn. Rather than baking a `ComposioClient` at construction time
//! (which would silently bypass a mid-session
//! [`crate::openhuman::config::ComposioConfig::mode`] toggle — see
//! issue #1710), each tool keeps an [`Arc<Config>`] and resolves the
//! client per call through
//! [`create_composio_client`] so a user flip from
//! `mode = "backend"` to `mode = "direct"` is honoured on the next
//! tool invocation without restarting the session. Mirrors the agent-
//! tool migration in
//! [`super::tools::ComposioExecuteTool`].

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::client::{create_composio_client, direct_execute, ComposioClientKind};
use super::providers::ToolScope;
use super::tools::resolve_action_scope;
use crate::openhuman::agent::harness::current_sandbox_mode;
use crate::openhuman::agent::harness::definition::SandboxMode;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult};

/// A single Composio action exposed as a first-class tool.
pub struct ComposioActionTool {
    /// Held instead of a pre-baked [`super::client::ComposioClient`] so
    /// the [`crate::openhuman::config::ComposioConfig::mode`] toggle is
    /// honoured on every invocation.
    ///
    /// Pre-fix this field was `client: ComposioClient`, which captured
    /// the backend-bound handle at sub-agent spawn time. Toggling
    /// `composio.mode = "direct"` mid-session invalidated other caches
    /// but left these per-action tools still routing through
    /// `staging-api.tinyhumans.ai/agent-integrations/composio/execute`
    /// — silently bypassing the direct-mode user's personal Composio
    /// tenant. Resolving the client per call via
    /// [`create_composio_client`] keeps dispatch in lockstep with the
    /// live config, matching
    /// [`super::tools::ComposioExecuteTool`]. See issue #1710.
    config: Arc<Config>,
    /// Action slug as-shipped to Composio, e.g. `"GMAIL_SEND_EMAIL"`.
    action_name: String,
    /// Human-readable description from the Composio tool-list response.
    description: String,
    /// Full JSON schema for the action's parameters. Falls back to
    /// `{"type":"object"}` when the upstream response omits it so the
    /// LLM still gets a valid (if loose) shape.
    parameters: Value,
}

impl ComposioActionTool {
    pub fn new(
        config: Arc<Config>,
        action_name: String,
        description: String,
        parameters: Option<Value>,
    ) -> Self {
        let parameters = parameters.unwrap_or_else(|| serde_json::json!({"type": "object"}));
        Self {
            config,
            action_name,
            description,
            parameters,
        }
    }
}

#[async_trait]
impl Tool for ComposioActionTool {
    fn name(&self) -> &str {
        &self.action_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        self.parameters.clone()
    }

    fn permission_level(&self) -> PermissionLevel {
        // Conservative default: many actions mutate external state
        // (send mail, create issues, modify calendars). Match
        // ComposioExecuteTool's write-level treatment so channel
        // permission caps behave identically whether the model goes
        // through the dispatcher or a per-action tool.
        PermissionLevel::Write
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Skill
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        // Agent-level sandbox gate (issue #685, CodeRabbit follow-up on
        // PR #904) — mirrors the check in
        // [`super::tools::ComposioExecuteTool::execute`] so a read-only
        // agent cannot slip a mutating call through the per-action
        // surface. The dispatcher path (`composio_execute`) and this
        // per-action path are the only two routes to the Composio
        // backend; both must honour the same invariant. Today no
        // read-only agent spawns per-action tools (only
        // `integrations_agent` registers them and it is
        // `sandbox_mode = "none"`), so this is strict defense-in-depth
        // for any future configuration that pairs the two.
        if matches!(current_sandbox_mode(), Some(SandboxMode::ReadOnly)) {
            let scope = resolve_action_scope(&self.action_name).await;
            if matches!(scope, ToolScope::Write | ToolScope::Admin) {
                tracing::info!(
                    tool = %self.action_name,
                    scope = scope.as_str(),
                    "[composio][sandbox] per-action execute blocked: agent is read-only, action is {}",
                    scope.as_str()
                );
                return Ok(ToolResult::error(format!(
                    "{}: action is classified `{}` and is refused because the calling \
                     agent is in strict read-only mode. Only `read`-scoped actions are \
                     available to this agent.",
                    self.action_name,
                    scope.as_str()
                )));
            }
        }

        // Resolve the client through the mode-aware factory on every
        // call so a direct-mode toggle takes effect immediately
        // (#1710). The pre-baked-client variant of this code routed all
        // executions through the backend tinyhumans tenant regardless
        // of mode — silently breaking direct mode for tool execution.
        // [#1710 Wave 4] Reload config fresh per execute so a mid-session
        // `composio.mode` toggle takes effect at the very next tool call.
        // The Arc<Config> snapshot held by `self` was taken at agent-init
        // time and is otherwise stale relative to subsequent set_api_key /
        // clear_api_key RPCs.
        let live_config = match config_rpc::load_config_with_timeout().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    tool = %self.action_name,
                    error = %e,
                    "[composio] per-action execute: load_config failed"
                );
                return Ok(ToolResult::error(format!(
                    "{}: failed to load live config: {e}",
                    self.action_name
                )));
            }
        };
        let kind = match create_composio_client(&live_config) {
            Ok(kind) => kind,
            Err(e) => {
                tracing::warn!(
                    tool = %self.action_name,
                    error = %e,
                    "[composio] per-action execute: factory failed"
                );
                return Ok(ToolResult::error(format!("{}: {e}", self.action_name)));
            }
        };

        let started = std::time::Instant::now();
        let res = match kind {
            ComposioClientKind::Backend(client) => {
                tracing::debug!(
                    tool = %self.action_name,
                    "[composio] per-action execute: backend variant"
                );
                // Wrap with auth_retry so a stale tinyhumans-tenant
                // JWT gets refreshed-and-replayed once before surfacing
                // (upstream behaviour).
                super::auth_retry::execute_with_auth_retry(&client, &self.action_name, Some(args))
                    .await
            }
            ComposioClientKind::Direct(direct) => {
                tracing::debug!(
                    tool = %self.action_name,
                    "[composio] per-action execute: direct variant"
                );
                // Direct path skips auth_retry — see ComposioExecuteTool
                // for rationale (no backend refresh surface).
                direct_execute(
                    &direct,
                    &self.action_name,
                    Some(args),
                    &live_config.composio.entity_id,
                )
                .await
            }
        };
        let elapsed_ms = started.elapsed().as_millis() as u64;

        match res {
            Ok(resp) => {
                crate::core::event_bus::publish_global(
                    crate::core::event_bus::DomainEvent::ComposioActionExecuted {
                        tool: self.action_name.clone(),
                        success: resp.successful,
                        error: resp.error.clone(),
                        cost_usd: resp.cost_usd,
                        elapsed_ms,
                    },
                );
                // Mirror `ComposioExecuteTool::execute` (composio/tools.rs):
                // prefer the backend-rendered `markdownFormatted` for LLM
                // consumption when present, fall back to the raw JSON
                // envelope on absence or non-success. Keeps both routes
                // (dispatcher + per-action) consistent so the model sees
                // the same compact transcript regardless of which tool
                // surface integrations_agent picked.
                let body = if resp.successful {
                    match resp
                        .markdown_formatted
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        Some(md) => md.to_string(),
                        None => serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
                    }
                } else {
                    serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into())
                };
                Ok(ToolResult::success(body))
            }
            Err(e) => {
                crate::core::event_bus::publish_global(
                    crate::core::event_bus::DomainEvent::ComposioActionExecuted {
                        tool: self.action_name.clone(),
                        success: false,
                        error: Some(e.to_string()),
                        cost_usd: 0.0,
                        elapsed_ms,
                    },
                );
                Ok(ToolResult::error(format!("{}: {e}", self.action_name)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::harness::with_current_sandbox_mode;

    /// Build a minimal `Arc<Config>` with `composio.mode = "backend"`
    /// (the default). The sandbox gate runs *before* any HTTP call or
    /// factory resolve, so these tests never reach the network. Mirrors
    /// the helper in `tools_tests.rs`.
    fn fake_config() -> Arc<Config> {
        let tmp = tempfile::tempdir().expect("tempdir for fake_config");
        let mut config = Config::default();
        config.config_path = tmp.path().join("config.toml");
        // Leak the tempdir so the path remains valid for the test's
        // lifetime — `Config::config_path` is just used as a lookup key
        // here, not actually written to.
        std::mem::forget(tmp);
        Arc::new(config)
    }

    // Direct-mode coverage no longer constructs an `Arc<Config>` helper:
    // `ComposioActionTool::execute` reloads config via
    // `load_config_with_timeout()` per call (#1710 Wave 4), so direct-
    // mode tests persist an isolated `config.toml` under `TEST_ENV_LOCK`
    // (see `factory_routes_through_direct_when_mode_is_direct`) rather
    // than injecting one through the constructor.

    fn error_text(result: &ToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|c| match c {
                crate::openhuman::tools::traits::ToolContent::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[tokio::test]
    async fn sandbox_read_only_blocks_per_action_write_call() {
        let t = ComposioActionTool::new(
            fake_config(),
            "GMAIL_SEND_EMAIL".to_string(),
            "send a gmail message".to_string(),
            None,
        );
        let result = with_current_sandbox_mode(SandboxMode::ReadOnly, async {
            t.execute(serde_json::json!({})).await.unwrap()
        })
        .await;
        assert!(
            result.is_error,
            "per-action Write under read-only must error"
        );
        let msg = error_text(&result);
        assert!(msg.contains("strict read-only"), "got: {msg}");
        assert!(msg.contains("`write`"), "got: {msg}");
    }

    #[tokio::test]
    async fn sandbox_read_only_blocks_per_action_admin_call() {
        let t = ComposioActionTool::new(
            fake_config(),
            "GMAIL_DELETE_EMAIL".to_string(),
            "destructive".to_string(),
            None,
        );
        let result = with_current_sandbox_mode(SandboxMode::ReadOnly, async {
            t.execute(serde_json::json!({})).await.unwrap()
        })
        .await;
        assert!(result.is_error);
        let msg = error_text(&result);
        assert!(msg.contains("`admin`"), "got: {msg}");
    }

    #[tokio::test]
    async fn sandbox_unset_leaves_per_action_execute_to_downstream() {
        // Outside any `with_current_sandbox_mode` scope the task-local
        // is `None` and the gate is a no-op. The downstream factory
        // resolve still fails (no backend session token / no api key),
        // but never with the sandbox text.
        //
        // The sandbox gate is a no-op here, so dispatch falls through to
        // `load_config_with_timeout()` (#1710 Wave 4). Hold
        // `TEST_ENV_LOCK` and point `OPENHUMAN_WORKSPACE` at an
        // isolated, persisted config so this test neither reads the
        // dev's real config nor races the shared env var against the
        // other config-loading composio tests.
        use crate::openhuman::config::TEST_ENV_LOCK;
        let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let tmp = tempfile::tempdir().expect("tempdir");
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }

        let mut config = Config::default();
        config.config_path = tmp.path().join("config.toml");
        config.workspace_dir = tmp.path().join("workspace");
        config.save().await.expect("save fake config to disk");

        let t = ComposioActionTool::new(
            Arc::new(config),
            "GMAIL_SEND_EMAIL".to_string(),
            "send".to_string(),
            None,
        );
        let result = t.execute(serde_json::json!({})).await.unwrap();
        let msg = error_text(&result);
        assert!(
            !msg.contains("strict read-only"),
            "unset sandbox must never trigger the gate, got: {msg}"
        );

        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }

    // ── Factory routing (#1710) ──────────────────────────────────────
    //
    // Regression coverage for the bug fix: `ComposioActionTool` now
    // resolves its client per call rather than caching one at
    // construction time, so a mid-session `composio.mode` toggle is
    // honoured on the very next per-action execute.

    #[tokio::test]
    async fn factory_routes_through_backend_when_mode_is_backend() {
        // Default `Config` has `composio.mode = "backend"`. Without a
        // stored backend session token the factory returns
        // `Err("no backend session ...")`. Assert that the error text
        // points at the backend code path (not direct-mode or staging-
        // api), confirming the routing branch.
        //
        // Production `.execute(..)` calls `load_config_with_timeout()`
        // per call which reads from `~/.openhuman/config.toml` (or the
        // workspace pointed at by `OPENHUMAN_WORKSPACE`). To isolate
        // the test from the dev's real config we hold `TEST_ENV_LOCK`,
        // point `OPENHUMAN_WORKSPACE` at a tempdir, and persist the
        // test's `Config` to that tempdir's `config.toml` before
        // invoking the tool.
        use crate::openhuman::config::TEST_ENV_LOCK;
        let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let tmp = tempfile::tempdir().expect("tempdir");
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }

        let mut config = Config::default();
        config.config_path = tmp.path().join("config.toml");
        config.workspace_dir = tmp.path().join("workspace");
        config.save().await.expect("save fake config to disk");

        let tool = ComposioActionTool::new(
            Arc::new(config),
            "GMAIL_FETCH_EMAILS".to_string(),
            "read-shaped slug so sandbox/scope gates don't short-circuit \
             the dispatch site"
                .to_string(),
            None,
        );
        let result = tool.execute(serde_json::json!({})).await.unwrap();
        assert!(result.is_error, "no backend session must error");
        let msg = error_text(&result);
        assert!(
            msg.contains("backend") || msg.contains("session"),
            "expected backend-mode session error, got: {msg}"
        );
        assert!(
            !msg.contains("direct mode"),
            "backend-mode failure must not surface direct-mode artifacts: {msg}"
        );

        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }

    #[tokio::test]
    async fn factory_routes_through_direct_when_mode_is_direct() {
        // Direct-mode config with an inline api_key — factory resolves
        // to the `Direct` variant. The downstream call will fail when
        // it attempts to hit `backend.composio.dev` from the unit test
        // sandbox, but the error must come from the direct path, not
        // a backend session lookup.
        //
        // Production `.execute(..)` calls `load_config_with_timeout()`
        // per call which reads from disk — see the matching note on
        // `factory_routes_through_backend_when_mode_is_backend`.
        use crate::openhuman::config::TEST_ENV_LOCK;
        let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let tmp = tempfile::tempdir().expect("tempdir");
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }

        let mut config = Config::default();
        config.config_path = tmp.path().join("config.toml");
        config.workspace_dir = tmp.path().join("workspace");
        config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
        config.composio.api_key = Some("test-direct-key".to_string());
        config.save().await.expect("save fake config to disk");

        let tool = ComposioActionTool::new(
            Arc::new(config),
            "GMAIL_FETCH_EMAILS".to_string(),
            "read-shaped slug".to_string(),
            None,
        );
        let result = tool.execute(serde_json::json!({})).await.unwrap();
        // Direct-mode resolve succeeds → no `factory failed` error.
        // The error (if any) will come from the downstream HTTP call,
        // which is fine — we just need to confirm the dispatch routed
        // through the direct branch rather than the backend branch.
        let msg = error_text(&result);
        assert!(
            !msg.contains("no backend session") && !msg.contains("staging-api"),
            "direct-mode dispatch must not leak backend session / staging-api \
             artifacts: {msg}"
        );

        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }

    #[tokio::test]
    async fn mode_toggle_between_calls_is_observed() {
        // Regression test for #1710: building the tool once with one
        // mode and toggling the config mid-session must take effect on
        // the next execute. We can't trivially mutate an `Arc<Config>`
        // without `Arc::get_mut` (single ref), so we run the two halves
        // sequentially against two different on-disk configs and assert
        // each routes through its respective branch. This captures the
        // core structural property — that no client is baked at
        // construction time — and is faithful to production because
        // `.execute(..)` calls `load_config_with_timeout()` per call.
        //
        // The actual in-place mutation flow on the live system is:
        // RPC `composio.set_mode` writes config.toml, the
        // `ComposioConfigChanged` event invalidates the parent
        // session's `Arc<Config>`, and the next sub-agent spawn picks
        // up the fresh `Arc<Config>` from
        // `Config::load_or_init().await`. Here we simulate that by
        // rewriting `OPENHUMAN_WORKSPACE/config.toml` between the two
        // halves while holding `TEST_ENV_LOCK`.
        use crate::openhuman::config::TEST_ENV_LOCK;
        let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // ── Backend half ────────────────────────────────────────────
        let tmp_backend = tempfile::tempdir().expect("tempdir backend");
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp_backend.path());
        }
        let mut backend_config = Config::default();
        backend_config.config_path = tmp_backend.path().join("config.toml");
        backend_config.workspace_dir = tmp_backend.path().join("workspace");
        backend_config
            .save()
            .await
            .expect("save backend config to disk");

        let backend_tool = ComposioActionTool::new(
            Arc::new(backend_config),
            "GMAIL_FETCH_EMAILS".to_string(),
            "read-shaped slug".to_string(),
            None,
        );
        let backend_result = backend_tool.execute(serde_json::json!({})).await.unwrap();
        let backend_msg = error_text(&backend_result);
        // Backend tool's error must point at a backend session lookup.
        assert!(
            backend_msg.contains("backend") || backend_msg.contains("session"),
            "backend-mode tool should surface a backend session error, got: {backend_msg}"
        );

        // ── Direct half ─────────────────────────────────────────────
        let tmp_direct = tempfile::tempdir().expect("tempdir direct");
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp_direct.path());
        }
        let mut direct_config = Config::default();
        direct_config.config_path = tmp_direct.path().join("config.toml");
        direct_config.workspace_dir = tmp_direct.path().join("workspace");
        direct_config.composio.mode =
            crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
        direct_config.composio.api_key = Some("test-direct-key".to_string());
        direct_config
            .save()
            .await
            .expect("save direct config to disk");

        let direct_tool = ComposioActionTool::new(
            Arc::new(direct_config),
            "GMAIL_FETCH_EMAILS".to_string(),
            "read-shaped slug".to_string(),
            None,
        );
        let direct_result = direct_tool.execute(serde_json::json!({})).await.unwrap();
        let direct_msg = error_text(&direct_result);

        // Direct tool's error must NOT mention a backend session — the
        // smoking gun for the pre-fix bug would have been the
        // direct-mode tool surfacing
        // `staging-api.tinyhumans.ai` / `no backend session` because
        // the cached client was a backend handle.
        assert!(
            !direct_msg.contains("no backend session"),
            "direct-mode tool must not surface backend-session artifacts: {direct_msg}"
        );

        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }
}
