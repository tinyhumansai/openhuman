//! Agent-facing tools that proxy through the openhuman backend's
//! `/agent-integrations/composio/*` routes.
//!
//! These expose Composio capabilities to the autonomous agent loop
//! (discovery + execution) and to the CLI/RPC surface via the normal
//! `Tool` trait plumbing in [`crate::openhuman::tools`].
//!
//! The surface is intentionally small and model-friendly:
//!
//! | Tool name                     | Purpose                                                     |
//! | ----------------------------- | ----------------------------------------------------------- |
//! | `composio_list_toolkits`      | Inspect the server allowlist (e.g. `["gmail", "notion"]`)   |
//! | `composio_list_connections`   | See which accounts are already connected                    |
//! | `composio_authorize`          | Start an OAuth handoff for a toolkit, returns `connectUrl`  |
//! | `composio_list_tools`         | Discover available action slugs + their JSON schemas        |
//! | `composio_execute`            | Run a Composio action with `{tool, arguments}`              |
//!
//! The agent loop is expected to chain `composio_list_tools` →
//! `composio_execute` when it needs to use a new action. The full schema
//! is returned in `composio_list_tools`'s output so the model can pick
//! the right slug and supply valid arguments without a separate round
//! trip.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::agent::harness::current_sandbox_mode;
use crate::openhuman::agent::harness::definition::SandboxMode;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult};

use super::client::ComposioClient;
use super::providers::{
    catalog_for_toolkit, classify_unknown, find_curated, get_provider, load_user_scope_or_default,
    toolkit_from_slug, ToolScope, UserScopePref,
};

/// Decision returned by [`evaluate_tool_visibility`].
enum ToolDecision {
    /// Action is curated for this toolkit and user scope allows it.
    Allow,
    /// Action exists in the curated list but the user's scope blocks
    /// it. `scope` is the curated classification.
    BlockedByScope { scope: ToolScope },
    /// Action is not in the toolkit's curated whitelist (and the
    /// toolkit has one). Hidden / rejected.
    NotCurated,
    /// Toolkit has no curated catalog — pass through, but still gate by
    /// the user scope using the [`classify_unknown`] heuristic.
    PassthroughCheckScope { scope: ToolScope },
}

/// Resolve a Composio action slug to its [`ToolScope`] classification.
///
/// Prefers the toolkit's curated catalog when available (most accurate
/// — curated entries are hand-classified) and falls back to the
/// [`classify_unknown`] heuristic for un-curated toolkits. Unparseable
/// slugs default to `Write` so the sandbox gate errs on the side of
/// blocking rather than letting a potentially-mutating action slip
/// through uncategorised.
pub(super) async fn resolve_action_scope(slug: &str) -> ToolScope {
    let Some(toolkit) = toolkit_from_slug(slug) else {
        return ToolScope::Write;
    };
    let catalog = get_provider(&toolkit)
        .and_then(|p| p.curated_tools())
        .or_else(|| catalog_for_toolkit(&toolkit));
    if let Some(cat) = catalog {
        if let Some(entry) = find_curated(cat, slug) {
            return entry.scope;
        }
    }
    classify_unknown(slug)
}

/// Decide whether a Composio action slug should be visible / executable
/// for the current user, given the registered provider's curated list
/// (if any) and the user's stored scope preference.
async fn evaluate_tool_visibility(slug: &str) -> ToolDecision {
    let Some(toolkit) = toolkit_from_slug(slug) else {
        // Unparseable slug — let the backend return its own error.
        return ToolDecision::Allow;
    };
    let pref = load_user_scope_or_default(&toolkit).await;
    // Prefer a registered provider's curated list; fall back to the
    // static toolkit→catalog map so toolkits without a native provider
    // (e.g. github) still get whitelist enforcement.
    let catalog = get_provider(&toolkit)
        .and_then(|p| p.curated_tools())
        .or_else(|| catalog_for_toolkit(&toolkit));
    match catalog {
        Some(catalog) => match find_curated(catalog, slug) {
            Some(curated) if pref.allows(curated.scope) => ToolDecision::Allow,
            Some(curated) => ToolDecision::BlockedByScope {
                scope: curated.scope,
            },
            None => ToolDecision::NotCurated,
        },
        None => {
            let scope = classify_unknown(slug);
            if pref.allows(scope) {
                ToolDecision::PassthroughCheckScope { scope }
            } else {
                ToolDecision::BlockedByScope { scope }
            }
        }
    }
}

/// Filter a freshly-fetched [`super::types::ComposioToolsResponse`] in
/// place: drop tools that aren't curated for their toolkit and tools
/// whose scope is disabled in the user's pref.
async fn filter_list_tools_response(resp: &mut super::types::ComposioToolsResponse) {
    let before = resp.tools.len();
    // Compute keep/drop decisions sequentially (the await means we
    // can't fold this into a single sync `retain` closure). Then zip
    // each tool with its decision and collect the survivors — clearer
    // than juggling a parallel index alongside `Vec::retain`.
    let mut keep: Vec<bool> = Vec::with_capacity(before);
    for t in &resp.tools {
        let decision = evaluate_tool_visibility(&t.function.name).await;
        keep.push(matches!(
            decision,
            ToolDecision::Allow | ToolDecision::PassthroughCheckScope { .. }
        ));
    }
    let drained: Vec<_> = resp.tools.drain(..).collect();
    resp.tools = drained
        .into_iter()
        .zip(keep)
        .filter_map(|(tool, keep_it)| if keep_it { Some(tool) } else { None })
        .collect();
    let after = resp.tools.len();
    if after != before {
        tracing::debug!(
            before,
            after,
            dropped = before - after,
            "[composio][scopes] composio_list_tools filtered"
        );
    }
}

/// Format a user-facing error message for a scope-blocked execution.
fn scope_error_message(slug: &str, scope: ToolScope, pref: UserScopePref) -> String {
    format!(
        "composio_execute: action `{slug}` is classified `{}` and is disabled in your \
         current scope preferences (read={}, write={}, admin={}). Update the toolkit's \
         scope preference (composio_set_user_scopes) to enable this category.",
        scope.as_str(),
        pref.read,
        pref.write,
        pref.admin,
    )
}

// ── composio_list_toolkits ──────────────────────────────────────────

pub struct ComposioListToolkitsTool {
    client: ComposioClient,
}

impl ComposioListToolkitsTool {
    pub fn new(client: ComposioClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for ComposioListToolkitsTool {
    fn name(&self) -> &str {
        "composio_list_toolkits"
    }
    fn description(&self) -> &str {
        "List the Composio toolkits currently enabled on the backend allowlist. \
         Use this before calling composio_authorize or composio_list_tools to see what \
         is allowed (e.g. gmail, notion)."
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
    fn category(&self) -> ToolCategory {
        // Composio proxies to external SaaS (Gmail, Notion, …), so it
        // lives in the Skill category and is picked up by sub-agents
        // with `category_filter = "skill"`.
        ToolCategory::Skill
    }
    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        tracing::debug!("[composio] tool list_toolkits.execute");
        match self.client.list_toolkits().await {
            Ok(resp) => Ok(ToolResult::success(
                serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
            )),
            Err(e) => Ok(ToolResult::error(format!(
                "composio_list_toolkits failed: {e}"
            ))),
        }
    }
}

// ── composio_list_connections ───────────────────────────────────────

pub struct ComposioListConnectionsTool {
    client: ComposioClient,
}

impl ComposioListConnectionsTool {
    pub fn new(client: ComposioClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for ComposioListConnectionsTool {
    fn name(&self) -> &str {
        "composio_list_connections"
    }
    fn description(&self) -> &str {
        "List the user's **currently-connected** Composio integrations. \
         Only entries with status ACTIVE / CONNECTED are returned; pending, \
         revoked, or failed connections are filtered out. Use this to detect \
         newly-authorised integrations mid-session. Each entry has \
         {id, toolkit, status, createdAt}."
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Skill
    }
    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        tracing::debug!("[composio] tool list_connections.execute");
        match self.client.list_connections().await {
            Ok(mut resp) => {
                // Filter server-side-indistinguishable states here —
                // callers should only ever see integrations the user
                // can actually act on. Matches the same ACTIVE /
                // CONNECTED allowlist used by
                // `fetch_connected_integrations_uncached` so the tool
                // output and the prompt's Delegation Guide agree on
                // what counts as "connected".
                resp.connections.retain(|c| {
                    let status = c.status.trim();
                    status.eq_ignore_ascii_case("ACTIVE")
                        || status.eq_ignore_ascii_case("CONNECTED")
                });
                Ok(ToolResult::success(
                    serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
                ))
            }
            Err(e) => Ok(ToolResult::error(format!(
                "composio_list_connections failed: {e}"
            ))),
        }
    }
}

// ── composio_authorize ──────────────────────────────────────────────

pub struct ComposioAuthorizeTool {
    client: ComposioClient,
}

impl ComposioAuthorizeTool {
    pub fn new(client: ComposioClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for ComposioAuthorizeTool {
    fn name(&self) -> &str {
        "composio_authorize"
    }
    fn description(&self) -> &str {
        "Begin an OAuth handoff for a Composio toolkit. Returns a `connectUrl` \
         the user must open in a browser to authorize the integration, plus the \
         resulting `connectionId`. The toolkit must be in the backend allowlist."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "toolkit": {
                    "type": "string",
                    "description": "Toolkit slug, e.g. 'gmail' or 'notion'."
                }
            },
            "required": ["toolkit"],
            "additionalProperties": false
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Skill
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let toolkit = args
            .get("toolkit")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if toolkit.is_empty() {
            return Ok(ToolResult::error(
                "composio_authorize: 'toolkit' is required",
            ));
        }
        tracing::debug!(toolkit = %toolkit, "[composio] tool authorize.execute");
        match self.client.authorize(&toolkit).await {
            Ok(resp) => {
                crate::core::event_bus::publish_global(
                    crate::core::event_bus::DomainEvent::ComposioConnectionCreated {
                        toolkit: toolkit.clone(),
                        connection_id: resp.connection_id.clone(),
                        connect_url: resp.connect_url.clone(),
                    },
                );
                Ok(ToolResult::success(format!(
                    "Open this URL to connect {toolkit}: {}\n(connectionId: {})",
                    resp.connect_url, resp.connection_id
                )))
            }
            Err(e) => Ok(ToolResult::error(format!("composio_authorize failed: {e}"))),
        }
    }
}

// ── composio_list_tools ─────────────────────────────────────────────

pub struct ComposioListToolsTool {
    client: ComposioClient,
}

impl ComposioListToolsTool {
    pub fn new(client: ComposioClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for ComposioListToolsTool {
    fn name(&self) -> &str {
        "composio_list_tools"
    }
    fn description(&self) -> &str {
        "List Composio action tools available through the backend. Pass an optional \
         `toolkits` array to filter (e.g. [\"gmail\"]). The result is a JSON array of \
         OpenAI function-calling tool schemas; use the slug from `function.name` as the \
         `tool` argument when calling `composio_execute`."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "toolkits": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of toolkit slugs to filter by."
                }
            },
            "additionalProperties": false
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Skill
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let toolkits = args.get("toolkits").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        });
        tracing::debug!(?toolkits, "[composio] tool list_tools.execute");
        match self.client.list_tools(toolkits.as_deref()).await {
            Ok(mut resp) => {
                filter_list_tools_response(&mut resp).await;
                Ok(ToolResult::success(
                    serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
                ))
            }
            Err(e) => Ok(ToolResult::error(format!(
                "composio_list_tools failed: {e}"
            ))),
        }
    }
}

// ── composio_execute ────────────────────────────────────────────────

pub struct ComposioExecuteTool {
    client: ComposioClient,
}

impl ComposioExecuteTool {
    pub fn new(client: ComposioClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for ComposioExecuteTool {
    fn name(&self) -> &str {
        "composio_execute"
    }
    fn description(&self) -> &str {
        "Execute a Composio action by slug. `tool` is the action slug returned from \
         composio_list_tools (e.g. 'GMAIL_SEND_EMAIL'); `arguments` is an object that \
         conforms to that tool's parameter schema. Returns the provider result plus \
         cost (USD)."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tool": {
                    "type": "string",
                    "description": "Composio action slug, e.g. 'GMAIL_SEND_EMAIL'."
                },
                "arguments": {
                    "type": "object",
                    "description": "Action-specific arguments. Shape depends on the tool."
                }
            },
            "required": ["tool"],
            "additionalProperties": false
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        // Some composio actions send emails, create files, etc. — treat
        // as write-level to respect channel permission caps.
        PermissionLevel::Write
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Skill
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let tool = args
            .get("tool")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if tool.is_empty() {
            return Ok(ToolResult::error(
                "composio_execute: 'tool' is required (e.g. GMAIL_SEND_EMAIL)",
            ));
        }
        let arguments = args.get("arguments").cloned();
        tracing::debug!(tool = %tool, "[composio] tool execute.execute");

        // Agent-level sandbox gate (issue #685) — applies on top of the
        // user's scope preference below. When the currently-executing
        // agent declares `sandbox_mode = "read_only"` in its
        // `agent.toml`, we refuse to dispatch any Write- or Admin-scoped
        // composio action regardless of what the user's scope pref
        // allows, so a strictly-read-only agent (planner, critic,
        // morning_briefing, …) can never mutate user state via the
        // composio surface. `SandboxMode::None` / `Sandboxed` (and the
        // `None` task-local value used by direct CLI / JSON-RPC / unit
        // tests) pass through unchanged.
        if matches!(current_sandbox_mode(), Some(SandboxMode::ReadOnly)) {
            let scope = resolve_action_scope(&tool).await;
            if matches!(scope, ToolScope::Write | ToolScope::Admin) {
                tracing::info!(
                    tool = %tool,
                    scope = scope.as_str(),
                    "[composio][sandbox] execute blocked: agent is read-only, action is {}",
                    scope.as_str()
                );
                return Ok(ToolResult::error(format!(
                    "composio_execute: action `{tool}` is classified `{}` and is refused \
                     because the calling agent is in strict read-only mode. Only `read`-scoped \
                     actions are available to this agent.",
                    scope.as_str()
                )));
            }
        }

        // Enforce per-user scope preferences before delegating to backend.
        match evaluate_tool_visibility(&tool).await {
            ToolDecision::Allow | ToolDecision::PassthroughCheckScope { .. } => {}
            ToolDecision::BlockedByScope { scope } => {
                let toolkit = toolkit_from_slug(&tool).unwrap_or_default();
                let pref = load_user_scope_or_default(&toolkit).await;
                let msg = scope_error_message(&tool, scope, pref);
                tracing::info!(
                    tool = %tool,
                    toolkit = %toolkit,
                    scope = scope.as_str(),
                    "[composio][scopes] execute blocked by user scope pref"
                );
                return Ok(ToolResult::error(msg));
            }
            ToolDecision::NotCurated => {
                let toolkit = toolkit_from_slug(&tool).unwrap_or_default();
                tracing::info!(
                    tool = %tool,
                    toolkit = %toolkit,
                    "[composio][scopes] execute blocked: action not in curated whitelist"
                );
                return Ok(ToolResult::error(format!(
                    "composio_execute: action `{tool}` is not in the curated whitelist for \
                     toolkit `{toolkit}`. Use composio_list_tools to see available actions."
                )));
            }
        }

        let started = std::time::Instant::now();
        let res = self.client.execute_tool(&tool, arguments.clone()).await;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match res {
            Ok(mut resp) => {
                crate::core::event_bus::publish_global(
                    crate::core::event_bus::DomainEvent::ComposioActionExecuted {
                        tool: tool.clone(),
                        success: resp.successful,
                        error: resp.error.clone(),
                        cost_usd: resp.cost_usd,
                        elapsed_ms,
                    },
                );
                // Per-toolkit post-processing of the upstream payload
                // (e.g. gmail HTML → markdown). Only run on successful
                // responses; errors are passed through verbatim.
                if resp.successful {
                    super::providers::init_default_providers();
                    if let Some(toolkit) = toolkit_from_slug(&tool) {
                        if let Some(provider) = get_provider(&toolkit) {
                            tracing::trace!(
                                toolkit = toolkit.as_str(),
                                tool = tool.as_str(),
                                has_args = arguments.is_some(),
                                "[composio_execute] post-processing action result"
                            );
                            provider.post_process_action_result(
                                &tool,
                                arguments.as_ref(),
                                &mut resp.data,
                            );
                        }
                    }
                }
                Ok(ToolResult::success(
                    serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
                ))
            }
            Err(e) => {
                crate::core::event_bus::publish_global(
                    crate::core::event_bus::DomainEvent::ComposioActionExecuted {
                        tool: tool.clone(),
                        success: false,
                        error: Some(e.to_string()),
                        cost_usd: 0.0,
                        elapsed_ms,
                    },
                );
                Ok(ToolResult::error(format!("composio_execute failed: {e}")))
            }
        }
    }
}

// ── Bulk registration helper ────────────────────────────────────────

/// Build the full set of composio agent tools when the integrations
/// client is available and composio is enabled. Returns an empty vec
/// otherwise so callers can always `.extend(...)` unconditionally.
pub fn all_composio_agent_tools(config: &crate::openhuman::config::Config) -> Vec<Box<dyn Tool>> {
    let Some(client) = super::client::build_composio_client(config) else {
        tracing::debug!("[composio] agent tools not registered — disabled or missing credentials");
        return Vec::new();
    };
    // `ComposioClient` is `Clone` (the inner `IntegrationClient` is Arc'd),
    // so each tool gets a cheap clone of the handle directly.
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(ComposioListToolkitsTool::new(client.clone())),
        Box::new(ComposioListConnectionsTool::new(client.clone())),
        Box::new(ComposioAuthorizeTool::new(client.clone())),
        Box::new(ComposioListToolsTool::new(client.clone())),
        Box::new(ComposioExecuteTool::new(client)),
    ];
    tracing::debug!(count = tools.len(), "[composio] agent tools registered");
    tools
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
