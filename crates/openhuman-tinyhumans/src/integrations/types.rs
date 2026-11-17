//! Shared types for agent integration tools.

use serde::Deserialize;

// ─────────────────────────────────────────────────────────────────────────────
// Connected integrations (Composio toolkits)
// ─────────────────────────────────────────────────────────────────────────────

/// Identity of a single active connection within a toolkit.
///
/// Surfaced in the system prompt so the orchestrator can disambiguate
/// multiple accounts for the same toolkit (e.g. "work Gmail" vs
/// "personal Gmail") and pass the correct `connection_id` to the
/// execute pipeline.
#[derive(Debug, Clone)]
pub struct IntegrationConnection {
    /// Composio connection ID — passed to execute when the user/agent
    /// targets a specific account.
    pub connection_id: String,
    /// Human-readable label derived from the connection's identity
    /// fields: `account_email`, `workspace`, or `username` (first
    /// non-empty wins). `None` when identity hasn't been enriched yet.
    pub label: Option<String>,
    /// Whether this is the default connection for the toolkit (oldest
    /// active connection by `created_at`).
    pub is_default: bool,
}

/// An external integration (e.g. a Composio OAuth-backed toolkit)
/// surfaced in the system prompt so the orchestrator knows which
/// services are available — both **already connected** and **available
/// to authorize**.
#[derive(Debug, Clone)]
pub struct ConnectedIntegration {
    /// Toolkit slug, e.g. `"gmail"`, `"notion"`.
    pub toolkit: String,
    /// Human-readable one-line description of what this integration can do.
    pub description: String,
    /// Per-action catalogue (only populated when `connected == true`).
    pub tools: Vec<ConnectedIntegrationTool>,
    /// Per-action catalogue for actions that the toolkit **does** support but
    /// the user has **not** unlocked via their per-toolkit scope preferences.
    /// The prompt renderer surfaces these descriptively (name + one-line +
    /// which scope is missing) so the agent can honestly answer "do you have
    /// X?" with "yes, but you need to flip the {scope} toggle in
    /// Connections → {toolkit}" — instead of silently claiming the
    /// capability doesn't exist (which is what happens when the agent has
    /// zero awareness of pref-gated actions).
    ///
    /// The agent CANNOT directly invoke these (no `parameters` schema is
    /// exposed; the LLM lacks the function definition) and it cannot flip
    /// the gating scope itself — there is no agent-callable scope-elevate
    /// tool. Intended flow: agent sees a gated tool → tells the user what
    /// it does + names the `unlock_paths` from the data → the user toggles
    /// the scope in the Connections UI → on the next turn the action
    /// graduates from `gated_tools` to `tools` and becomes callable.
    pub gated_tools: Vec<GatedIntegrationTool>,
    /// Whether the user has an active OAuth connection for this
    /// toolkit. When `false`, the toolkit is in the backend allowlist
    /// but no authorization has been completed yet — `tools` is empty
    /// and the orchestrator must point the user at Settings instead of
    /// attempting to delegate.
    pub connected: bool,
    /// All active connections for this toolkit, sorted by `created_at`
    /// ascending (oldest first). The first entry is the default.
    /// Empty when `connected == false`.
    pub connections: Vec<IntegrationConnection>,
    /// Raw upstream connection status when a connection row exists but
    /// is not `ACTIVE` — e.g. `"INITIATED"`, `"INITIALIZING"`,
    /// `"FAILED"`, `"EXPIRED"`. `None` means either the user is
    /// `ACTIVE` (use `connected = true`) OR there is no connection
    /// row at all (truly disconnected).
    ///
    /// Distinguishes an OAuth flow still in flight, an expired token and
    /// a connection never started (issue #2365).
    pub non_active_status: Option<String>,
}

/// A toolkit action that exists in the catalog but is currently hidden from
/// the agent's callable function list because the user's scope preference
/// for this toolkit does not allow the action's required scope.
///
/// Deliberately no `parameters` field: the LLM should NOT be able to construct
/// a call envelope for a gated tool — it can only describe its existence and
/// point the user at the unlock path. The agent has no scope-elevate tool;
/// once the user toggles the gating scope in the Connections UI, the action
/// moves from `ConnectedIntegration.gated_tools` to `ConnectedIntegration.tools`
/// on the next prompt rebuild and becomes a real callable function.
#[derive(Debug, Clone)]
pub struct GatedIntegrationTool {
    /// Action slug, e.g. `"GMAIL_BATCH_DELETE_MESSAGES"`.
    pub name: String,
    /// One-line description of the action.
    pub description: String,
    /// Which scope the user must enable for this action to become callable.
    /// Lowercase: `"read"`, `"write"`, `"admin"`. The vast majority of gated
    /// rows are `"admin"` (destructive actions); `"write"` only appears for
    /// users who have explicitly turned write off, which is unusual.
    pub required_scope: String,
    /// Literal lines the agent should show the user, verbatim, when offering
    /// to unlock this action — one entry per available path (typically: the
    /// agent-side meta-tool, and the manual UI toggle). Populated at
    /// partition time in `composio::ops`. The prompt-side rule is "show
    /// these to the user, don't substitute your own framing" — keeping the
    /// text in the data (not in the system prompt) lets us tweak wording
    /// without invalidating the KV-cache prefix and avoids biasing the
    /// model toward a memorized template that drops options.
    pub unlock_paths: Vec<String>,
}

/// A single action available on a connected integration.
#[derive(Debug, Clone)]
pub struct ConnectedIntegrationTool {
    /// Action slug, e.g. `"GMAIL_SEND_EMAIL"`.
    pub name: String,
    /// One-line description of the action.
    pub description: String,
    /// JSON schema for the action's parameters. `None` when the backend
    /// didn't supply a schema.
    pub parameters: Option<serde_json::Value>,
}

// ── Pricing types (fetched from backend) ────────────────────────────

/// Per-integration pricing returned by `GET /agent-integrations/pricing`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct IntegrationPricing {
    #[serde(default)]
    pub integrations: PricingIntegrations,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PricingIntegrations {
    #[serde(default)]
    pub google_places: Option<IntegrationPricingEntry>,
    #[serde(default)]
    pub parallel: Option<IntegrationPricingEntry>,
    #[serde(default)]
    pub tinyfish: Option<IntegrationPricingEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IntegrationPricingEntry {
    #[serde(default)]
    pub available: bool,
    #[serde(default)]
    pub pricing: serde_json::Value,
}

// ── Backend response envelope ───────────────────────────────────────

/// Standard `{ success, data, error }` envelope from the backend.
#[derive(Debug, Deserialize)]
pub struct BackendResponse<T> {
    #[allow(dead_code)]
    pub success: bool,
    pub data: Option<T>,
    #[serde(default)]
    #[allow(dead_code)]
    pub error: Option<String>,
}
