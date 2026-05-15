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

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::agent::harness::current_sandbox_mode;
use crate::openhuman::agent::harness::definition::SandboxMode;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{
    PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolResult,
};

use super::client::{
    create_composio_client, direct_execute, direct_list_connections, ComposioClientKind,
};
use super::providers::{
    catalog_for_toolkit, classify_unknown, find_curated, get_provider, load_user_scope_or_default,
    toolkit_from_slug, ToolScope, UserScopePref,
};
use super::types::ComposioToolsResponse;

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

/// Drop tools whose toolkit is not in `connected` (case-insensitive).
/// Returns the number of dropped tools so callers can log it.
/// `toolkit_from_slug` already lowercases its result, so the comparison
/// is direct against entries the caller has already lowercased.
fn retain_connected_tools(
    resp: &mut super::types::ComposioToolsResponse,
    connected: &std::collections::HashSet<String>,
) -> usize {
    let before = resp.tools.len();
    resp.tools.retain(|t| {
        toolkit_from_slug(&t.function.name)
            .map(|tk| connected.contains(&tk))
            .unwrap_or(false)
    });
    before - resp.tools.len()
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

/// One-line description: collapse whitespace + truncate.
fn one_line(desc: &str, max_chars: usize) -> String {
    let collapsed: String = desc.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        collapsed
    } else {
        let snippet: String = collapsed.chars().take(max_chars).collect();
        format!("{snippet}…")
    }
}

/// Pull required + optional top-level argument names from a JSON Schema
/// `parameters` object. Returns `(required, optional)` — both empty when
/// the schema is missing or doesn't follow the expected shape.
fn split_arg_names(parameters: Option<&Value>) -> (Vec<String>, Vec<String>) {
    let Some(params) = parameters.and_then(Value::as_object) else {
        return (Vec::new(), Vec::new());
    };
    let required: Vec<String> = params
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut optional: Vec<String> = params
        .get("properties")
        .and_then(Value::as_object)
        .map(|props| props.keys().cloned().collect())
        .unwrap_or_default();
    optional.retain(|k| !required.contains(k));
    (required, optional)
}

/// Compact markdown rendering of `composio_list_tools` output.
///
/// Drops the full JSON parameter schemas (the main token cost) and keeps
/// only what the agent needs to pick a slug and call `composio_execute`:
/// the slug, a one-line description, and the names of required +
/// optional top-level arguments. Tools are grouped by toolkit prefix.
fn render_tools_markdown(resp: &super::types::ComposioToolsResponse) -> String {
    use std::collections::BTreeMap;
    use std::fmt::Write as _;

    if resp.tools.is_empty() {
        return "_No composio tools available._".to_string();
    }

    // Group by toolkit slug (lowercase prefix). Use BTreeMap for stable
    // ordering so the agent sees the same shape across calls.
    let mut by_toolkit: BTreeMap<String, Vec<&super::types::ComposioToolSchema>> = BTreeMap::new();
    for t in &resp.tools {
        let toolkit = toolkit_from_slug(&t.function.name).unwrap_or_else(|| "other".to_string());
        by_toolkit.entry(toolkit).or_default().push(t);
    }

    let mut out = format!(
        "# Composio tools ({} actions across {} toolkit{})\n\n\
         Call `composio_execute` with `tool=<SLUG>` and an `arguments` object \
         matching the listed parameters.\n",
        resp.tools.len(),
        by_toolkit.len(),
        if by_toolkit.len() == 1 { "" } else { "s" },
    );

    for (toolkit, tools) in &by_toolkit {
        let _ = writeln!(out, "\n## {toolkit}");
        for t in tools {
            let desc = t
                .function
                .description
                .as_deref()
                .map(|d| one_line(d, 160))
                .unwrap_or_default();
            let (required, optional) = split_arg_names(t.function.parameters.as_ref());
            let _ = write!(out, "- `{}`", t.function.name);
            if !desc.is_empty() {
                let _ = write!(out, " — {desc}");
            }
            if !required.is_empty() {
                let _ = write!(out, " **req:** {}", required.join(", "));
            }
            if !optional.is_empty() {
                let _ = write!(out, " **opt:** {}", optional.join(", "));
            }
            out.push('\n');
        }
    }
    out
}

// `execute_direct` was previously defined locally here; it now lives
// in `super::client::direct_execute` so the ops.rs RPC handler and the
// agent-tool path share a single direct-mode envelope reshaper.
// See `direct_execute`'s rustdoc for the v3 → ComposioExecuteResponse
// translation contract.

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
    /// Held instead of a pre-baked `ComposioClient` so the
    /// [`crate::openhuman::config::ComposioConfig::mode`] toggle is
    /// honoured on every call (see [`ComposioExecuteTool`] doc for the
    /// bug this guards against — #1710).
    config: Arc<Config>,
}

impl ComposioListToolkitsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
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
        // Mirror the mode-aware pattern in
        // `ops::composio_list_toolkits`. In direct mode there is no
        // server-side allowlist; the user's personal Composio account
        // governs availability, so we return an empty toolkits list
        // with an explanatory log instead of silently routing through
        // the backend tinyhumans tenant (#1710).
        // [#1710 Wave 4] Reload config fresh per execute so a mid-session
        // `composio.mode` toggle takes effect at the very next tool call.
        // The Arc<Config> snapshot held by `self` was taken at agent-init
        // time and is otherwise stale relative to subsequent set_api_key /
        // clear_api_key RPCs.
        let live_config = match config_rpc::load_config_with_timeout().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "[composio] tool: load_config failed");
                return Ok(ToolResult::error(format!(
                    "composio: failed to load live config: {e}"
                )));
            }
        };
        let client = match create_composio_client(&live_config) {
            Ok(ComposioClientKind::Backend(client)) => {
                tracing::debug!("[composio] list_toolkits.execute: backend variant");
                client
            }
            Ok(ComposioClientKind::Direct(_)) => {
                tracing::info!(
                    "[composio-direct] list_toolkits.execute: direct mode active — \
                     returning empty toolkits list. Users manage available toolkits \
                     via app.composio.dev."
                );
                let resp = super::types::ComposioToolkitsResponse::default();
                return Ok(ToolResult::success(
                    serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
                ));
            }
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "composio_list_toolkits failed: {e}"
                )));
            }
        };
        match client.list_toolkits().await {
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
    /// Held instead of a pre-baked `ComposioClient` so the
    /// [`crate::openhuman::config::ComposioConfig::mode`] toggle is
    /// honoured on every call (#1710).
    config: Arc<Config>,
}

impl ComposioListConnectionsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
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
        // Mirror `ops::composio_list_connections`: route through the mode-aware
        // factory so the agent sees the correct tenant's connections in both
        // backend and direct mode. Before this fix, direct mode returned an
        // empty list regardless of the user's actual Composio connections,
        // which caused the agent to incorrectly conclude that no integrations
        // were linked and prompt unnecessary re-authorization (#1710).
        // [#1710 Wave 4] Reload config fresh per execute so a mid-session
        // `composio.mode` toggle takes effect at the very next tool call.
        // The Arc<Config> snapshot held by `self` was taken at agent-init
        // time and is otherwise stale relative to subsequent set_api_key /
        // clear_api_key RPCs.
        let live_config = match config_rpc::load_config_with_timeout().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "[composio] list_connections.execute: load_config failed");
                return Ok(ToolResult::error(format!(
                    "composio_list_connections: failed to load live config: {e}"
                )));
            }
        };
        let mut resp = match create_composio_client(&live_config) {
            Ok(ComposioClientKind::Backend(client)) => {
                tracing::debug!("[composio] list_connections.execute: backend variant");
                client.list_connections().await.map_err(|e| {
                    anyhow::anyhow!("composio_list_connections (backend) failed: {e}")
                })?
            }
            Ok(ComposioClientKind::Direct(direct)) => {
                tracing::debug!("[composio-direct] list_connections.execute: direct variant");
                direct_list_connections(&direct).await.map_err(|e| {
                    anyhow::anyhow!("composio_list_connections (direct) failed: {e}")
                })?
            }
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "composio_list_connections failed: {e}"
                )));
            }
        };
        // Filter server-side-indistinguishable states — callers should only
        // see integrations the user can actually act on. Matches the same
        // ACTIVE/CONNECTED allowlist used by `fetch_connected_integrations_uncached`
        // so the tool output and the prompt's Delegation Guide agree on what
        // counts as "connected".
        resp.connections.retain(|c| c.is_active());
        tracing::debug!(
            count = resp.connections.len(),
            "[composio] list_connections.execute: returning active connections"
        );
        Ok(ToolResult::success(
            serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
        ))
    }
}

// ── composio_authorize ──────────────────────────────────────────────

pub struct ComposioAuthorizeTool {
    /// Held instead of a pre-baked `ComposioClient` so the
    /// [`crate::openhuman::config::ComposioConfig::mode`] toggle is
    /// honoured on every call (#1710).
    config: Arc<Config>,
}

impl ComposioAuthorizeTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
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
        // Resolve per call so a live mode toggle is honoured. In
        // direct mode the OAuth handoff is performed by the user's
        // personal Composio tenant via app.composio.dev rather than
        // the backend's `/agent-integrations/composio/authorize`
        // route, so we refuse this verb explicitly instead of
        // silently routing through the wrong tenant.
        // [#1710 Wave 4] Reload config fresh per execute so a mid-session
        // `composio.mode` toggle takes effect at the very next tool call.
        // The Arc<Config> snapshot held by `self` was taken at agent-init
        // time and is otherwise stale relative to subsequent set_api_key /
        // clear_api_key RPCs.
        let live_config = match config_rpc::load_config_with_timeout().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "[composio] tool: load_config failed");
                return Ok(ToolResult::error(format!(
                    "composio: failed to load live config: {e}"
                )));
            }
        };
        let client = match create_composio_client(&live_config) {
            Ok(ComposioClientKind::Backend(client)) => {
                tracing::debug!("[composio] authorize.execute: backend variant");
                client
            }
            Ok(ComposioClientKind::Direct(_)) => {
                tracing::info!(
                    toolkit = %toolkit,
                    "[composio-direct] authorize.execute: direct mode active — \
                     refusing backend OAuth handoff. Connect this toolkit via \
                     app.composio.dev for the personal Composio tenant."
                );
                return Ok(ToolResult::error(format!(
                    "composio_authorize: direct mode is active. Connect `{toolkit}` \
                     through your personal Composio account at app.composio.dev \
                     instead of the backend OAuth flow."
                )));
            }
            Err(e) => {
                return Ok(ToolResult::error(format!("composio_authorize failed: {e}")));
            }
        };
        match client.authorize(&toolkit, None).await {
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
    /// Held instead of a pre-baked `ComposioClient` so the
    /// [`crate::openhuman::config::ComposioConfig::mode`] toggle is
    /// honoured on every call. Resolving the client per call mirrors
    /// [`crate::openhuman::composio::ops::composio_execute`] and avoids
    /// the staged-routing bug (#1710) where a long-lived backend client
    /// would survive a user switch into `direct` mode.
    config: Arc<Config>,
}

impl ComposioListToolsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ComposioListToolsTool {
    fn name(&self) -> &str {
        "composio_list_tools"
    }
    fn description(&self) -> &str {
        "List Composio action tools available through the backend. By default only \
         actions for toolkits the user has actively connected are returned — pass \
         `include_unconnected=true` to see every allowlisted toolkit's actions \
         (useful when planning whether to call `composio_authorize` for a new toolkit). \
         Pass an optional `toolkits` array to further filter (e.g. [\"gmail\"]). The \
         result is a JSON object with a `tools` array of OpenAI function-calling \
         tool schemas; use the slug from each entry's `function.name` as the `tool` \
         argument when calling `composio_execute`."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "toolkits": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of toolkit slugs to filter by."
                },
                "include_unconnected": {
                    "type": "boolean",
                    "description": "When true, include actions from toolkits the user \
                                    has not connected yet. Defaults to false (only \
                                    connected toolkits)."
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
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }

    async fn execute_with_options(
        &self,
        args: Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        let toolkits = args.get("toolkits").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        });
        let include_unconnected = args
            .get("include_unconnected")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        tracing::debug!(
            ?toolkits,
            include_unconnected,
            prefer_markdown = options.prefer_markdown,
            "[composio] tool list_tools.execute"
        );

        // Resolve the client through the mode-aware factory so a
        // direct-mode user does not silently get the backend
        // tinyhumans-tenant tool list. In direct mode we return an
        // empty `tools` array with an explanatory log, mirroring the
        // ops.rs `composio_list_toolkits` / `composio_list_connections`
        // pattern. Surfacing the empty list explicitly is correct
        // fail-mode: the alternative — falling through to the backend
        // path — is exactly the bug we're closing (#1710).
        // [#1710 Wave 4] Reload config fresh per execute so a mid-session
        // `composio.mode` toggle takes effect at the very next tool call.
        // The Arc<Config> snapshot held by `self` was taken at agent-init
        // time and is otherwise stale relative to subsequent set_api_key /
        // clear_api_key RPCs.
        let live_config = match config_rpc::load_config_with_timeout().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "[composio] tool: load_config failed");
                return Ok(ToolResult::error(format!(
                    "composio: failed to load live config: {e}"
                )));
            }
        };
        let client = match create_composio_client(&live_config) {
            Ok(ComposioClientKind::Backend(client)) => {
                tracing::debug!("[composio] list_tools.execute: backend variant");
                client
            }
            Ok(ComposioClientKind::Direct(_)) => {
                tracing::info!(
                    "[composio-direct] list_tools.execute: direct mode active — \
                     returning empty tools list. Discovery is delegated to the user's \
                     personal Composio account; backend-tenant tools are intentionally \
                     NOT surfaced in direct mode."
                );
                let resp = ComposioToolsResponse::default();
                let mut result = ToolResult::success(
                    serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
                );
                if options.prefer_markdown {
                    result.markdown_formatted = Some(render_tools_markdown(&resp));
                }
                return Ok(result);
            }
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "composio_list_tools failed: {e}"
                )));
            }
        };

        match client.list_tools(toolkits.as_deref()).await {
            Ok(mut resp) => {
                filter_list_tools_response(&mut resp).await;

                if !include_unconnected {
                    // Restrict to toolkits with an ACTIVE / CONNECTED
                    // account. Mirrors the same status allowlist used by
                    // composio_list_connections so this view and the
                    // prompt's Delegation Guide stay in sync.
                    match client.list_connections().await {
                        Ok(conns) => {
                            let connected: std::collections::HashSet<String> = conns
                                .connections
                                .iter()
                                .filter(|c| c.is_active())
                                .map(|c| c.normalized_toolkit())
                                .filter(|t| !t.is_empty())
                                .collect();
                            let dropped = retain_connected_tools(&mut resp, &connected);
                            tracing::debug!(
                                connected_toolkits = connected.len(),
                                dropped,
                                kept = resp.tools.len(),
                                "[composio] list_tools restricted to connected toolkits"
                            );
                        }
                        Err(e) => {
                            // Soft-fail: surface the issue to the agent
                            // so it can retry with include_unconnected
                            // rather than silently returning [].
                            return Ok(ToolResult::error(format!(
                                "composio_list_tools failed to fetch connections \
                                 (needed to filter to connected toolkits — pass \
                                 include_unconnected=true to skip this check): {e}"
                            )));
                        }
                    }
                }

                let mut result = ToolResult::success(
                    serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
                );
                if options.prefer_markdown {
                    result.markdown_formatted = Some(render_tools_markdown(&resp));
                }
                Ok(result)
            }
            Err(e) => Ok(ToolResult::error(format!(
                "composio_list_tools failed: {e}"
            ))),
        }
    }

    fn supports_markdown(&self) -> bool {
        true
    }
}

// ── composio_execute ────────────────────────────────────────────────

pub struct ComposioExecuteTool {
    /// Held instead of a pre-baked `ComposioClient` so the
    /// [`crate::openhuman::config::ComposioConfig::mode`] toggle is
    /// honoured on every call.
    ///
    /// The earlier shape stored a backend-bound `ComposioClient` baked
    /// at agent boot. When the user toggled
    /// `composio.mode = "direct"` mid-session the
    /// `ComposioConfigChanged` event invalidated caches, but this tool's
    /// pre-baked client kept routing executions through
    /// `staging-api.tinyhumans.ai/agent-integrations/composio/execute`
    /// — silently bypassing the direct-mode user's personal Composio
    /// tenant. Resolving the client per call via
    /// [`create_composio_client`] keeps dispatch in lockstep with the
    /// live config, matching
    /// [`crate::openhuman::composio::ops::composio_execute`]. See
    /// issue #1710.
    config: Arc<Config>,
}

impl ComposioExecuteTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
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
                tracing::warn!(error = %e, "[composio] tool execute.execute: load_config failed");
                return Ok(ToolResult::error(format!(
                    "composio_execute: failed to load live config: {e}"
                )));
            }
        };
        let kind = match create_composio_client(&live_config) {
            Ok(kind) => kind,
            Err(e) => {
                tracing::warn!(error = %e, "[composio] tool execute.execute: factory failed");
                return Ok(ToolResult::error(format!("composio_execute failed: {e}")));
            }
        };

        let started = std::time::Instant::now();
        let res = match kind {
            ComposioClientKind::Backend(client) => {
                tracing::debug!(tool = %tool, "[composio] tool execute.execute: backend variant");
                // Backend path retains the upstream `auth_retry` wrapper
                // so a 401 from a stale tinyhumans-tenant token is
                // refreshed-and-replayed exactly once before surfacing.
                super::auth_retry::execute_with_auth_retry(&client, &tool, arguments).await
            }
            ComposioClientKind::Direct(direct) => {
                tracing::debug!(tool = %tool, "[composio] tool execute.execute: direct variant");
                // Direct path skips `auth_retry`: the user's stored
                // Composio API key doesn't have a backend-side refresh
                // surface and a 401 here is a config issue (rotated key,
                // wrong key, deleted) that should surface to the user
                // rather than retry-loop.
                direct_execute(&direct, &tool, arguments, &live_config.composio.entity_id).await
            }
        };
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match res {
            Ok(resp) => {
                crate::core::event_bus::publish_global(
                    crate::core::event_bus::DomainEvent::ComposioActionExecuted {
                        tool: tool.clone(),
                        success: resp.successful,
                        error: resp.error.clone(),
                        cost_usd: resp.cost_usd,
                        elapsed_ms,
                    },
                );
                // Prefer the backend-rendered markdown when available
                // (tinyhumansai/backend#683). The backend handles parsing
                // for all composio actions; if a tool isn't formatted
                // server-side `markdown_formatted` is None and we fall
                // back to the raw JSON envelope.
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
    // Registration gate: ask the mode-aware probe "can this user call
    // composio at all?" — true when EITHER a backend session token OR a
    // stored/inline direct-mode API key is present. The pre-fix path
    // called `build_composio_client(...).is_none()`, which is
    // backend-only and silently dropped the 5 generic agent tools for
    // direct-mode users (#1710). Per-action dispatch inside each tool
    // re-resolves through the factory so the live `composio.mode`
    // toggle keeps winning.
    if !crate::openhuman::agent::harness::subagent_runner::user_is_signed_in_to_composio(config) {
        tracing::debug!(
            "[composio] agent tools not registered — user is not signed in to composio \
             (no backend session and no direct API key)"
        );
        return Vec::new();
    }
    // All five tools resolve their client per call through the
    // mode-aware factory; they only need a handle to the live root
    // config to do so. Sharing one `Arc<Config>` keeps the registration
    // cheap (no repeated `Config::clone` walks) and ensures every tool
    // sees the same live snapshot.
    let config_arc = Arc::new(config.clone());
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(ComposioListToolkitsTool::new(config_arc.clone())),
        Box::new(ComposioListConnectionsTool::new(config_arc.clone())),
        Box::new(ComposioAuthorizeTool::new(config_arc.clone())),
        Box::new(ComposioListToolsTool::new(config_arc.clone())),
        Box::new(ComposioExecuteTool::new(config_arc)),
    ];
    tracing::debug!(count = tools.len(), "[composio] agent tools registered");
    tools
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
