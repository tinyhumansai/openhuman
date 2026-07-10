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
//! Scope elevation (read/write/admin) is deliberately NOT an agent tool;
//! the user must toggle it themselves in the Connections UI.
//!
//! The agent loop is expected to chain `composio_list_tools` →
//! `composio_execute` when it needs to use a new action. The full schema
//! is returned in `composio_list_tools`'s output so the model can pick
//! the right slug and supply valid arguments without a separate round
//! trip.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::agent::harness::current_sandbox_mode;
use crate::openhuman::agent::harness::current_task_recency_window;
use crate::openhuman::agent::harness::definition::SandboxMode;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{
    PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolResult,
};

use super::client::{create_composio_client, direct_list_connections, ComposioClientKind};
use super::providers::{
    catalog_for_toolkit, classify_unknown, find_curated, get_provider, load_user_scope_or_default,
    toolkit_from_slug, ToolScope, UserScopePref,
};
use super::types::ComposioToolsResponse;

mod direct;

pub use direct::{ComposioAction, ComposioConnectedAccount, ComposioTool};

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
    connected: &HashSet<String>,
) -> usize {
    let before = resp.tools.len();
    resp.tools.retain(|t| {
        toolkit_from_slug(&t.function.name)
            .map(|tk| connected.contains(&tk))
            .unwrap_or(false)
    });
    before - resp.tools.len()
}

fn normalized_scope_toolkits(
    requested: Option<&[String]>,
    connected: Option<&HashSet<String>>,
) -> Vec<String> {
    let mut out = BTreeSet::new();
    if let Some(requested) = requested {
        for toolkit in requested {
            let normalized = toolkit.trim().to_ascii_lowercase();
            if !normalized.is_empty() {
                out.insert(normalized);
            }
        }
    } else if let Some(connected) = connected {
        out.extend(connected.iter().filter(|t| !t.is_empty()).cloned());
    }
    out.into_iter().collect()
}

fn uncatalogued_toolkits(toolkits: &[String]) -> Vec<String> {
    toolkits
        .iter()
        .filter(|toolkit| {
            get_provider(toolkit)
                .and_then(|provider| provider.curated_tools())
                .or_else(|| catalog_for_toolkit(toolkit))
                .is_none()
        })
        .cloned()
        .collect()
}

fn empty_uncurated_toolkits_message(toolkits: &[String]) -> Option<String> {
    let unsupported = uncatalogued_toolkits(toolkits);
    if unsupported.is_empty() {
        return None;
    }
    let names = unsupported
        .iter()
        .map(|toolkit| format!("`{toolkit}`"))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "composio_list_tools: no agent-ready actions are available for toolkit(s) {names}. \
         These integrations can be connected, but OpenHuman does not yet ship curated agent \
         tool catalogs for them. Use a supported toolkit such as Google Drive or Google Sheets \
         for now, or try again after catalog support lands."
    ))
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
///
/// Embeds the unlock path in the error itself so the agent reads the
/// instruction straight off the tool response — same policy-in-data
/// approach as the `gated_tools` surface. Only ONE path: the user
/// toggles the scope in the Connections UI. The agent has no tool to
/// flip scopes (see the note above the removed `ComposioEnableScopeTool`
/// for why) — it can only describe the gate and point at the UI.
fn scope_error_message(slug: &str, scope: ToolScope, pref: UserScopePref) -> String {
    let toolkit = toolkit_from_slug(slug).unwrap_or_default();
    let scope_str = scope.as_str();
    format!(
        "composio_execute: action `{slug}` is classified `{scope_str}` and is \
         disabled in the user's current scope preferences for `{toolkit}` \
         (read={}, write={}, admin={}). Tell the user this action requires the \
         `{scope_str}` scope and they can enable it themselves in \
         **Connections → {toolkit} → {scope_str}**. Do not claim you can flip \
         it — you cannot.",
        pref.read, pref.write, pref.admin,
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
        // lives in the Workflow category and is picked up by sub-agents
        // with `category_filter = "skill"`.
        ToolCategory::Workflow
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
        // Anchor the reload to this tool's original config path rather
        // than re-resolving process-global `OPENHUMAN_WORKSPACE`; the
        // tool is scoped to the user/workspace it was created for.
        let live_config =
            match config_rpc::reload_config_snapshot_with_timeout(self.config.as_ref()).await {
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
        ToolCategory::Workflow
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
        // Anchor the reload to this tool's original config path rather
        // than re-resolving process-global `OPENHUMAN_WORKSPACE`; the
        // tool is scoped to the user/workspace it was created for.
        let live_config = match config_rpc::reload_config_snapshot_with_timeout(
            self.config.as_ref(),
        )
        .await
        {
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
                    // [#1166 / Sentry TAURI-RUST-X9] Symmetric error
                    // routing with `ops.rs::composio_list_connections`.
                    // The agent-tool path can also fire 401s when a
                    // direct-mode user has a bad API key — without this
                    // hook the failure escapes the classifier and lands
                    // as an unclassified Sentry event. Render WITH the
                    // `[composio-direct]` anchor BEFORE reporting so the
                    // classifier arm in `is_provider_user_state_message`
                    // (gated on that prefix) actually fires.
                    let rendered = format!(
                        "[composio-direct] composio_list_connections (direct) failed: {e:#}"
                    );
                    super::ops::report_composio_op_error("list_connections", &rendered);
                    anyhow::anyhow!("{rendered}")
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
        ToolCategory::Workflow
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
        // Anchor the reload to this tool's original config path rather
        // than re-resolving process-global `OPENHUMAN_WORKSPACE`; the
        // tool is scoped to the user/workspace it was created for.
        let live_config =
            match config_rpc::reload_config_snapshot_with_timeout(self.config.as_ref()).await {
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

// ── composio_connect (inline approval card, #3993) ──────────────────

/// Canonicalize an agent/user-supplied toolkit slug to the form Composio's
/// backend expects. Mirrors `canonicalizeComposioToolkitSlug` on the FE
/// (`app/src/lib/composio/toolkitSlug.ts`) — **keep the alias maps in sync**.
/// The agent frequently guesses `google_drive` where Composio uses
/// `googledrive` (#3993); without this the OAuth handoff fails with an opaque
/// error.
fn canonicalize_toolkit_slug(slug: &str) -> String {
    let key = slug.trim().to_ascii_lowercase();
    match key.as_str() {
        "feishu" | "lark" => "larksuite".to_string(),
        "google_calendar" => "googlecalendar".to_string(),
        "google_drive" => "googledrive".to_string(),
        "google_sheets" => "googlesheets".to_string(),
        _ => key,
    }
}

/// Fresh (uncached) liveness check for `toolkit`.
///
/// Tri-state via `Result`:
/// - `Ok(true)`  — a connection is verified ACTIVE.
/// - `Ok(false)` — the read succeeded but no ACTIVE connection exists.
/// - `Err(_)`    — the state could **not** be verified (client construction or
///   the list call failed).
///
/// Used to confirm liveness after the approval gate resolves `Allow`. The
/// card-driven path only approves once it has polled the connection ACTIVE,
/// but other approval surfaces (a typed `yes`, Telegram's approval prompt, or
/// an existing auto-approve entry) resolve `Allow` with no OAuth poll — so
/// `Allow` alone must NOT be treated as "connected" (#3993, codex review).
///
/// Distinguishing `Err` from `Ok(false)` lets the caller fail closed on a
/// transient backend/auth failure **without** fabricating an "OAuth not
/// complete" reason that wrongly blames the user (#4062, coderabbit review).
async fn connection_is_active(config: &Config, toolkit: &str) -> anyhow::Result<bool> {
    let active_match = |connections: &[super::types::ComposioConnection]| {
        connections
            .iter()
            .any(|c| c.is_active() && c.normalized_toolkit().eq_ignore_ascii_case(toolkit))
    };
    match create_composio_client(config)? {
        ComposioClientKind::Backend(client) => {
            Ok(active_match(&client.list_connections().await?.connections))
        }
        ComposioClientKind::Direct(direct) => Ok(active_match(
            &direct_list_connections(&direct).await?.connections,
        )),
    }
}

/// Connect a Composio integration **inline in the chat** instead of
/// sending the user off to Settings → Connections.
///
/// Unlike [`ComposioAuthorizeTool`] (which hands the agent a raw
/// `connectUrl` it is not allowed to paste), this tool raises an
/// approval card via the process-global `ApprovalGate`. The frontend
/// recognises `tool_name == "composio_connect"` and renders a **Connect**
/// button: clicking it runs the existing `composio_authorize` RPC, opens
/// the OAuth handoff, and polls `composio_list_connections` until the
/// toolkit is ACTIVE — at which point it resolves the gate. The agent
/// then resumes the original task in the same turn.
pub struct ComposioConnectTool {
    config: Arc<Config>,
}

impl ComposioConnectTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ComposioConnectTool {
    fn name(&self) -> &str {
        "composio_connect"
    }
    fn description(&self) -> &str {
        "Connect a Composio integration (OAuth) for the user **inline in the chat**. \
         Raises an approval card with a Connect button — the user authorizes in one \
         click without leaving the conversation, and this tool returns once the \
         connection is active (or the user declines). ALWAYS prefer this over telling \
         the user to open Settings → Connections. Returns {toolkit, connected}."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "toolkit": {
                    "type": "string",
                    "description": "Toolkit slug to connect, e.g. 'gmail' or 'notion'."
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
        ToolCategory::Workflow
    }
    // NOTE: `external_effect` deliberately stays `false`. Gating happens
    // *inside* `execute` via a manual `ApprovalGate` intercept so we can
    // (a) skip the card when the toolkit is already connected and (b) carry
    // the toolkit slug into the card for the inline Connect button. The
    // engine's auto-gate is unconditional and would double-prompt.
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let raw_toolkit = args
            .get("toolkit")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if raw_toolkit.is_empty() {
            return Ok(ToolResult::error("composio_connect: 'toolkit' is required"));
        }
        // Canonicalize before any backend call — the agent often guesses
        // `google_drive` where Composio expects `googledrive` (#3993).
        let toolkit = canonicalize_toolkit_slug(raw_toolkit);
        tracing::debug!(raw = %raw_toolkit, toolkit = %toolkit, "[composio] tool connect.execute");

        // The inline connect card only has a surface on an interactive chat
        // turn (the web-chat path installs `APPROVAL_CHAT_CONTEXT`). On
        // background / cron turns there is no UI to click Connect, so fail
        // closed with a clear message rather than parking forever — mirrors
        // the `install_tool` guard (#3993).
        if crate::openhuman::approval::APPROVAL_CHAT_CONTEXT
            .try_with(|_| ())
            .is_err()
        {
            return Ok(ToolResult::error(format!(
                "[policy-denied] composio_connect needs an interactive chat turn. \
                 Ask the user to connect '{toolkit}' in Settings → Connections."
            )));
        }

        // Reload config per call so a mid-session `composio.mode` toggle is
        // honoured (#1710), then skip the card entirely if the toolkit is
        // already connected — avoids a flash of a Connect card that would
        // immediately resolve.
        let live_config =
            match config_rpc::reload_config_snapshot_with_timeout(self.config.as_ref()).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(error = %e, "[composio] connect.execute: load_config failed");
                    self.config.as_ref().clone()
                }
            };
        let already_connected = super::fetch_connected_integrations(&live_config)
            .await
            .into_iter()
            .any(|ci| ci.connected && ci.toolkit.eq_ignore_ascii_case(&toolkit));
        if already_connected {
            tracing::debug!(toolkit = %toolkit, "[composio] connect.execute: already connected");
            return Ok(ToolResult::success(serde_json::to_string(&json!({
                "toolkit": toolkit,
                "connected": true,
                "already_connected": true,
            }))?));
        }

        // Validate the toolkit against the *connectable* catalog before raising
        // a card. The orchestrator only knows which toolkits are already
        // connected — it must NOT confabulate "unsupported" from that list
        // (#3993). This grounds the answer: a backend-allowlisted toolkit gets
        // a card; a genuinely unsupported one gets a clear, listed refusal
        // instead of a card that would fail on Connect.
        match create_composio_client(&live_config) {
            Ok(ComposioClientKind::Backend(client)) => {
                if let Ok(resp) = client.list_toolkits().await {
                    // Empty allowlist = backend predates the catalog / unknown;
                    // don't block — let the OAuth handoff report support.
                    if !resp.toolkits.is_empty()
                        && !resp
                            .toolkits
                            .iter()
                            .any(|s| s.eq_ignore_ascii_case(&toolkit))
                    {
                        let available = resp.toolkits.join(", ");
                        tracing::info!(toolkit = %toolkit, "[composio] connect.execute: toolkit not in allowlist");
                        return Ok(ToolResult::error(format!(
                            "composio_connect: '{toolkit}' is not an available integration. \
                             Connectable toolkits: {available}"
                        )));
                    }
                }
            }
            Ok(ComposioClientKind::Direct(_)) => {
                // Personal-tenant (direct) mode performs OAuth at app.composio.dev,
                // not via the backend handoff the card drives — so an inline card
                // can't complete it. Point the user to Settings instead.
                return Ok(ToolResult::error(format!(
                    "composio_connect: direct Composio mode is active — connect '{toolkit}' \
                     in Settings → Connections (your personal Composio account)."
                )));
            }
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "composio_connect: Composio is unavailable: {e}"
                )));
            }
        }

        // Raise the inline connect card via the approval gate. The frontend
        // resolves it with `approve_once` once it polls the connection ACTIVE;
        // an explicit decline or the 10-minute TTL resolves it as denied.
        let gate = match crate::openhuman::approval::ApprovalGate::try_global() {
            Some(g) => g,
            None => {
                return Ok(ToolResult::error(
                    "composio_connect: approval gate unavailable in this environment",
                ));
            }
        };
        let summary = format!("Connect {toolkit} to complete your task");
        let (outcome, _request_id) = gate
            .intercept_audited("composio_connect", &summary, json!({ "toolkit": toolkit }))
            .await;
        match outcome {
            crate::openhuman::approval::GateOutcome::Allow => {
                // `Allow` only means the prompt was approved — re-check liveness
                // with a fresh read, because non-card approval surfaces (typed
                // "yes", Telegram, auto-approve) resolve Allow without running
                // the OAuth poll (#3993, codex review).
                match connection_is_active(&live_config, &toolkit).await {
                    Ok(true) => {
                        tracing::debug!(toolkit = %toolkit, "[composio] connect.execute: connection active");
                        Ok(ToolResult::success(serde_json::to_string(&json!({
                            "toolkit": toolkit,
                            "connected": true,
                        }))?))
                    }
                    Ok(false) => {
                        tracing::info!(toolkit = %toolkit, "[composio] connect.execute: approved but not yet active");
                        Ok(ToolResult::success(serde_json::to_string(&json!({
                            "toolkit": toolkit,
                            "connected": false,
                            "reason": "Approved, but the connection is not active yet — the user still needs to complete the OAuth flow.",
                        }))?))
                    }
                    Err(e) => {
                        // Couldn't verify liveness — a transient backend/auth
                        // failure, NOT proof the user skipped OAuth. Fail closed
                        // (connected:false) but report a verification error
                        // rather than fabricating an "OAuth incomplete" reason
                        // that blames the user and can drive the agent into
                        // reconnect loops (#4062, coderabbit review).
                        tracing::warn!(toolkit = %toolkit, error = %e, "[composio] connect.execute: liveness check failed");
                        Ok(ToolResult::success(serde_json::to_string(&json!({
                            "toolkit": toolkit,
                            "connected": false,
                            "reason": "Approved, but the connection state could not be verified right now (a temporary problem reaching Composio). Please try connecting again in a moment.",
                        }))?))
                    }
                }
            }
            crate::openhuman::approval::GateOutcome::Deny { reason } => {
                tracing::info!(toolkit = %toolkit, reason = %reason, "[composio] connect.execute: declined");
                Ok(ToolResult::success(serde_json::to_string(&json!({
                    "toolkit": toolkit,
                    "connected": false,
                    "declined": true,
                    "reason": reason,
                }))?))
            }
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
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional Composio action tags to filter by \
                                    (OR semantics — multiple tags broaden the result, \
                                    e.g. [\"readOnlyHint\"] or [\"repos\", \"stars\"]). \
                                    Case-insensitive."
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
        ToolCategory::Workflow
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
        // tags is only forwarded to the backend when the request is explicitly
        // scoped to GitHub — it is the one toolkit where the backend honours the
        // param (other toolkits ignore it and passing it could cause unintended
        // filtering on future toolkit expansions).
        let raw_tags = args.get("tags").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        });
        let tags = if super::ops::should_forward_tags(toolkits.as_deref()) {
            raw_tags
        } else {
            None
        };
        let include_unconnected = args
            .get("include_unconnected")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        tracing::debug!(
            ?toolkits,
            ?tags,
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
        // Anchor the reload to this tool's original config path rather
        // than re-resolving process-global `OPENHUMAN_WORKSPACE`; the
        // tool is scoped to the user/workspace it was created for.
        let live_config =
            match config_rpc::reload_config_snapshot_with_timeout(self.config.as_ref()).await {
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

        match client
            .list_tools(toolkits.as_deref(), tags.as_deref())
            .await
        {
            Ok(mut resp) => {
                filter_list_tools_response(&mut resp).await;
                let mut connected_toolkits: Option<HashSet<String>> = None;

                if !include_unconnected {
                    // Restrict to toolkits with an ACTIVE / CONNECTED
                    // account. Mirrors the same status allowlist used by
                    // composio_list_connections so this view and the
                    // prompt's Delegation Guide stay in sync.
                    match client.list_connections().await {
                        Ok(conns) => {
                            let connected: HashSet<String> = conns
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
                            connected_toolkits = Some(connected);
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

                if resp.tools.is_empty() {
                    let scoped_toolkits =
                        normalized_scope_toolkits(toolkits.as_deref(), connected_toolkits.as_ref());
                    if let Some(message) = empty_uncurated_toolkits_message(&scoped_toolkits) {
                        tracing::debug!(
                            toolkits = ?scoped_toolkits,
                            "[composio] list_tools empty for uncurated toolkit scope"
                        );
                        return Ok(ToolResult::error(message));
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
                },
                "connection_id": {
                    "type": "string",
                    "description": "Optional. Target a specific account when multiple are connected for a toolkit. Use the connection_id from '## Connected Integrations'. Omit to use the default account."
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
        ToolCategory::Workflow
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
        let connection_id = args
            .get("connection_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        // INFO-level entry log — visible without bumping log level. Logs
        // ARG KEYS only by default so traces don't leak email bodies / PII;
        // the full arg JSON goes through a paired DEBUG log below for deep
        // debugging. Together these let `grep "[composio][execute]"` show
        // the full agent→backend audit trail at default verbosity.
        let arg_keys: Vec<&str> = arguments
            .as_ref()
            .and_then(|v| v.as_object())
            .map(|obj| obj.keys().map(|k| k.as_str()).collect())
            .unwrap_or_default();
        tracing::info!(
            tool = %tool,
            connection_id = %connection_id,
            arg_keys = ?arg_keys,
            "[composio][execute] >> dispatch"
        );
        if let Some(ref args_json) = arguments {
            tracing::debug!(
                tool = %tool,
                args = %args_json,
                "[composio][execute] >> full args (DEBUG)"
            );
        }

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

        // Inject `timeZone` / `singleEvents` defaults for Google
        // Calendar list slugs so the host's IANA zone reaches the API
        // regardless of how the model built the args (issue #1714).
        // No-op for every other slug; respects caller-supplied values.
        let iana = super::googlecalendar_args::current_iana_timezone();
        tracing::debug!(
            target: "composio",
            slug = %tool,
            iana = %iana,
            "[composio][dispatcher] applying calendar query defaults pre-dispatch"
        );
        let arguments =
            super::googlecalendar_args::apply_calendar_query_defaults(&tool, arguments, &iana);

        // Task-recency window (morning briefing): when the calling agent
        // installed a window, inject best-effort server-side narrowing for
        // curated task-fetch slugs. No-op for every other slug and for
        // normal chat / CLI / JSON-RPC (window unset → None). The
        // authoritative enforcement is the post-filter on the response below.
        let task_window_since = current_task_recency_window().map(|w| {
            chrono::Utc::now()
                - chrono::Duration::from_std(w).unwrap_or_else(|_| chrono::Duration::zero())
        });
        let arguments = match task_window_since {
            Some(since) => super::task_window::apply_window_args(&tool, arguments, since),
            None => arguments,
        };

        // Resolve the client through the mode-aware factory on every
        // call so a direct-mode toggle takes effect immediately
        // (#1710). The pre-baked-client variant of this code routed all
        // executions through the backend tinyhumans tenant regardless
        // of mode — silently breaking direct mode for tool execution.
        // [#1710 Wave 4] Reload config fresh per execute so a mid-session
        // `composio.mode` toggle takes effect at the very next tool call.
        // Anchor the reload to this tool's original config path rather
        // than re-resolving process-global `OPENHUMAN_WORKSPACE`; the
        // tool is scoped to the user/workspace it was created for.
        let live_config = match config_rpc::reload_config_snapshot_with_timeout(
            self.config.as_ref(),
        )
        .await
        {
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
        // Centralized prepare → retry → error-mapping pipeline (#1797),
        // mode-aware over the backend/direct split (#1710).
        let res = super::execute_dispatch::execute_composio_action_kind(
            kind,
            &tool,
            arguments,
            &live_config.composio.entity_id,
        )
        .await;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match res {
            Ok(resp) => {
                // Authoritative task-recency enforcement: drop task rows older
                // than the window. No-op unless a window is installed AND the
                // slug is a curated task-fetch action. Runs before the
                // markdown/JSON body decision so the agent reads filtered data.
                let resp = match task_window_since {
                    Some(since) => super::task_window::filter_response(&tool, resp, since),
                    None => resp,
                };
                tracing::info!(
                    tool = %tool,
                    successful = resp.successful,
                    error = ?resp.error,
                    elapsed_ms,
                    cost_usd = resp.cost_usd,
                    "[composio][execute] << result"
                );
                tracing::debug!(
                    tool = %tool,
                    response = ?resp,
                    "[composio][execute] << full response (DEBUG)"
                );
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
                tracing::warn!(
                    tool = %tool,
                    error = %e,
                    elapsed_ms,
                    "[composio][execute] << dispatch error"
                );
                crate::core::event_bus::publish_global(
                    crate::core::event_bus::DomainEvent::ComposioActionExecuted {
                        tool: tool.clone(),
                        success: false,
                        error: Some(e.to_string()),
                        cost_usd: 0.0,
                        elapsed_ms,
                    },
                );
                Ok(ToolResult::error(e))
            }
        }
    }
}

// NOTE: A `composio_enable_scope` agent-callable meta-tool used to live
// here. It was removed deliberately: scope elevation is a
// security-sensitive, cross-session state change that unlocks
// destructive actions, and putting that flip behind LLM-mediated
// "user consent" both (a) made the safety contract depend on model
// behavior — the weakest place for it — and (b) was a soft gate the
// model could route around (e.g. trash-via-label). The user must
// toggle scopes themselves in **Connections → {toolkit} → {scope}
// row**; the agent only describes the gated capability and points at
// that UI path.
//
// Two surfaces name this policy; keep them in sync:
//   - `GatedIntegrationTool.unlock_paths` populated in `composio::ops`
//   - `scope_error_message` returned from `composio_execute` blocks

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
        // Inline-in-chat OAuth connect card (#3993). Raises an approval card
        // with a Connect button instead of handing the agent a raw URL.
        Box::new(ComposioConnectTool::new(config_arc.clone())),
        Box::new(ComposioListToolsTool::new(config_arc.clone())),
        Box::new(ComposioExecuteTool::new(config_arc)),
        // Pref-elevation is intentionally NOT an agent-callable tool;
        // the user must flip it themselves in the Connections UI.
        // See the long comment above the (removed) ComposioEnableScopeTool.
    ];
    tracing::debug!(count = tools.len(), "[composio] agent tools registered");
    tools
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
