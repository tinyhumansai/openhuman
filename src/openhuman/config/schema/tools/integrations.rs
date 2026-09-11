//! Composio, secrets, computer control, and agent integration toggle types.

use super::super::defaults;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Composio integration routing mode for the main backend-proxied flow.
///
/// `"backend"` (default) — every Composio call (toolkits, connections,
/// authorize, tools, execute, triggers, …) is proxied through the
/// OpenHuman backend (`api.tinyhumans.ai/agent-integrations/composio/*`).
/// The backend owns the Composio API key, allowlist, billing/margin, and
/// HMAC-verified trigger webhooks fanned out over socket.io.
///
/// `"direct"` — the core hits `https://backend.composio.dev/api/v{2,3}`
/// directly with the user's own Composio API key (BYO). Tool execution is
/// synchronous and works fully sovereign. Real-time **trigger webhooks**
/// (the async push surface that the backend currently mediates via
/// socket.io) do not work in direct mode — the user has to enable them
/// out-of-band on Composio's dashboard and configure their own webhook
/// sink. See `composio/tools/direct.rs` for the underlying client.
pub const COMPOSIO_MODE_BACKEND: &str = "backend";
pub const COMPOSIO_MODE_DIRECT: &str = "direct";

fn default_composio_mode() -> String {
    COMPOSIO_MODE_BACKEND.into()
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ComposioConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_entity_id")]
    pub entity_id: String,
    /// When true, the triage pipeline is disabled for all Composio
    /// triggers. Triggers are still recorded to history.
    /// Overrides `triage_disabled_toolkits` when set.
    #[serde(default)]
    pub triage_disabled: bool,
    /// Per-toolkit triage opt-out list. Toolkit slugs listed here
    /// skip the LLM triage turn — triggers are still recorded to
    /// history. Case-insensitive match against the incoming toolkit
    /// field (e.g. `["gmail", "slack"]`).
    #[serde(default)]
    pub triage_disabled_toolkits: Vec<String>,

    /// Routing mode for the main Composio integration flow. One of
    /// [`COMPOSIO_MODE_BACKEND`] (default — proxied through the OpenHuman
    /// backend) or [`COMPOSIO_MODE_DIRECT`] (BYO API key, calls
    /// `backend.composio.dev` directly).
    ///
    /// The user-provided API key for direct mode is *not* stored in the
    /// TOML — it lives in the encrypted keychain via
    /// [`crate::openhuman::security::credentials`] under the
    /// `composio-direct` provider slot. We only persist the mode here so
    /// the factory can pick the right client at construction time.
    #[serde(default = "default_composio_mode")]
    pub mode: String,

    /// **Deprecated for direct storage** — present so users that hand-edit
    /// `config.toml` can drop the key in here. The factory still prefers
    /// the keychain-backed value over this field. Default `None`.
    #[serde(default)]
    pub api_key: Option<String>,
}

fn default_entity_id() -> String {
    "default".into()
}

impl Default for ComposioConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            entity_id: default_entity_id(),
            triage_disabled: false,
            triage_disabled_toolkits: Vec::new(),
            mode: default_composio_mode(),
            api_key: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SecretsConfig {
    #[serde(default = "defaults::default_true")]
    pub encrypt: bool,
}

impl Default for SecretsConfig {
    fn default() -> Self {
        Self {
            encrypt: defaults::default_true(),
        }
    }
}

// ── Agent integration tools (backend-proxied) ───────────────────────

/// Routing mode for an integration that supports a backend-managed
/// default and an optional BYO ("bring your own API key") override.
pub const INTEGRATION_MODE_MANAGED: &str = "managed";
pub const INTEGRATION_MODE_BYO: &str = "byo";

fn default_integration_mode() -> String {
    INTEGRATION_MODE_MANAGED.into()
}

/// Per-integration toggle.
///
/// Defaults to **OpenHuman-managed** routing: the OpenHuman backend
/// owns the upstream API key, billing, and rate limits — the user only
/// has to flip `enabled` to make the tools available.
///
/// Users who hold their own provider account can switch `mode` to
/// `"byo"` and supply `api_key`. In that case tools register **iff**
/// the integration is `enabled = true` **and** `api_key` is a non-empty
/// trimmed string — see [`IntegrationToggle::is_active`]. This mirrors
/// the rule the Settings UI surfaces to the user ("loaded iff API key
/// is provided and enabled").
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct IntegrationToggle {
    #[serde(default = "defaults::default_true")]
    pub enabled: bool,
    /// Routing mode. One of [`INTEGRATION_MODE_MANAGED`] (default — the
    /// OpenHuman backend proxies the call) or [`INTEGRATION_MODE_BYO`]
    /// (the user's own API key is required and tools refuse to
    /// register without it).
    #[serde(default = "default_integration_mode")]
    pub mode: String,
    /// API key for [`INTEGRATION_MODE_BYO`]. Ignored in managed mode.
    /// Trimmed empty / `None` ⇒ no BYO key configured.
    #[serde(default)]
    pub api_key: Option<String>,
}

impl IntegrationToggle {
    /// Returns true when the integration should be wired up at tool-
    /// registration time. Managed mode requires only `enabled`; BYO
    /// mode requires both `enabled` and a non-empty `api_key`.
    pub fn is_active(&self) -> bool {
        if !self.enabled {
            return false;
        }
        match self.mode.as_str() {
            INTEGRATION_MODE_BYO => self
                .api_key
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false),
            _ => true,
        }
    }
}

impl Default for IntegrationToggle {
    fn default() -> Self {
        Self {
            enabled: defaults::default_true(),
            mode: default_integration_mode(),
            api_key: None,
        }
    }
}

/// Agent integration tools that proxy through the backend API.
///
/// The backend URL and auth token are **not** configurable here —
/// they're always resolved from the core `config.api_url` plus the
/// app-session JWT.
/// Composio in particular is unconditionally enabled and has no toggle:
/// as long as the user is signed in, composio tools are available.
///
/// The per-tool `apify`, `twilio`, `google_places`, `parallel`, and `tinyfish`
/// flags below are preserved because those integrations incur per-call
/// costs that the user may legitimately want to turn off; composio
/// costs are metered server-side, so there is no client-side toggle
/// for it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct IntegrationsConfig {
    /// Twilio phone-call integration.
    #[serde(default)]
    pub twilio: IntegrationToggle,

    /// Google Places location search integration.
    #[serde(default)]
    pub google_places: IntegrationToggle,

    /// Parallel web search & content extraction integration.
    #[serde(default)]
    pub parallel: IntegrationToggle,

    /// TinyFish web search, fetch, and browser automation integration.
    #[serde(default)]
    pub tinyfish: IntegrationToggle,

    /// Stock-price / market-data integration (Alpha Vantage on the backend).
    #[serde(default)]
    pub stock_prices: IntegrationToggle,
}

#[cfg(test)]
#[path = "integrations_integration_toggle_tests_tests.rs"]
mod integration_toggle_tests;
