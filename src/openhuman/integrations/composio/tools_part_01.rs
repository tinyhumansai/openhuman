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
    catalog_for_toolkit, classify_unknown, find_curated, toolkit_from_slug, ToolScope,
    UserScopePref,
};
use super::ops::load_user_scope_pref;
use super::types::ComposioToolsResponse;

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
    resolve_action_scope_sync(slug)
}

/// Synchronous core used by policy hooks that cannot await.
fn resolve_action_scope_sync(slug: &str) -> ToolScope {
    let Some(toolkit) = toolkit_from_slug(slug) else {
        return ToolScope::Write;
    };
    let catalog = catalog_for_toolkit(&toolkit);
    if let Some(cat) = catalog {
        if let Some(entry) = find_curated(cat, slug) {
            return entry.scope;
        }
    }
    classify_unknown(slug)
}

/// Whether an action must pass through the human approval gate.
pub(super) fn action_mutates_external_state(slug: &str) -> bool {
    matches!(
        resolve_action_scope_sync(slug),
        ToolScope::Write | ToolScope::Admin
    )
}

/// Decide whether a Composio action slug should be visible / executable
/// for the current user, given the registered provider's curated list
/// (if any) and the user's stored scope preference.
async fn evaluate_tool_visibility(config: &Config, slug: &str) -> ToolDecision {
    let Some(toolkit) = toolkit_from_slug(slug) else {
        // Unparseable slug — let the backend return its own error.
        return ToolDecision::Allow;
    };
    let pref = load_user_scope_pref(config, &toolkit).await;
    // The catalog covers every catalogued toolkit directly now — the
    // engine's `get_provider(toolkit).curated_tools()` hop this used to
    // prefer was pure indirection, verified identical to `catalog_for_toolkit`
    // for every toolkit that had a native provider.
    let catalog = catalog_for_toolkit(&toolkit);
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
        .filter(|toolkit| catalog_for_toolkit(toolkit).is_none())
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
async fn filter_list_tools_response(
    config: &Config,
    resp: &mut super::types::ComposioToolsResponse,
) {
    let before = resp.tools.len();
    // Compute keep/drop decisions sequentially (the await means we
    // can't fold this into a single sync `retain` closure). Then zip
    // each tool with its decision and collect the survivors — clearer
    // than juggling a parallel index alongside `Vec::retain`.
    let mut keep: Vec<bool> = Vec::with_capacity(before);
    for t in &resp.tools {
        let decision = evaluate_tool_visibility(config, &t.function.name).await;
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
                crate::core::bus::BUS.publish(
                    crate::core::events::DomainEvent::ComposioConnectionCreated {
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

/// Default bound (seconds) for how long [`ComposioConnectTool`] parks on the
/// inline-connect approval card before giving up (issue #4756).
///
/// The gate's own TTL is up to ten minutes (`DEFAULT_APPROVAL_TTL` in
/// `approval::gate`). That is fine when a human is watching the card, but when
/// the card can't be resolved — a headless/eval run, or a chat turn whose
/// client has since disconnected — `composio_connect` would otherwise block the
/// whole turn for minutes and deliver an empty reply, while the read path
/// (`composio_list_connections`) returns a graceful "not connected" prompt in
/// seconds. Bounding the park keeps the interactive resume-in-turn UX for a
/// present user (a click + OAuth round-trip completes well inside it) while
/// guaranteeing the act path degrades to a fast connect prompt instead of
/// hanging. Generous by design; env-overridable, `0` restores the full gate TTL.
const DEFAULT_COMPOSIO_CONNECT_TIMEOUT_SECS: u64 = 120;
