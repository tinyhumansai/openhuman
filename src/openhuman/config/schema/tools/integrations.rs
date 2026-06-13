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
    /// [`crate::openhuman::credentials`] under the
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

// ── Native computer control (mouse + keyboard) ─────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct ComputerControlConfig {
    /// Master toggle for mouse and keyboard tools. Disabled by default —
    /// the user must explicitly opt in.
    #[serde(default)]
    pub enabled: bool,
    /// Opt-in for the mutating `ax_interact` actions (`press` / `set_value`).
    /// Disabled by default: the read-only `list` action is always available,
    /// but actuating arbitrary app controls / typing into arbitrary fields
    /// requires explicit user opt-in (mirrors `enabled` for mouse/keyboard).
    #[serde(default)]
    pub ax_interact_mutations: bool,
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

fn default_polymarket_gamma_base_url() -> String {
    "https://gamma-api.polymarket.com".into()
}

fn default_polymarket_clob_base_url() -> String {
    "https://clob.polymarket.com".into()
}

fn default_polymarket_timeout_secs() -> u64 {
    15
}

fn default_polymarket_enabled() -> bool {
    false
}

fn default_polymarket_polygon_rpc_url() -> String {
    "https://polygon-rpc.com".into()
}

fn default_polymarket_usdc_contract() -> String {
    "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174".into()
}

fn default_polymarket_clob_exchange_contract() -> String {
    "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E".into()
}

/// Polymarket CLOB L2 credentials (api_key + HMAC secret + passphrase).
///
/// Single source of truth for both the config TOML surface AND the
/// in-process HTTP signing path — `polymarket.rs` / `clob_auth.rs` use
/// this type directly so there is no parallel internal struct + From-impl
/// glue to keep in sync.
#[derive(Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct PolymarketClobCredentials {
    pub api_key: String,
    pub secret: String,
    pub passphrase: String,
}

impl PolymarketClobCredentials {
    /// Returns true iff all three credential fields are non-empty after
    /// trimming whitespace.
    pub fn is_complete(&self) -> bool {
        !(self.api_key.trim().is_empty()
            || self.secret.trim().is_empty()
            || self.passphrase.trim().is_empty())
    }
}

impl std::fmt::Debug for PolymarketClobCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PolymarketClobCredentials")
            .field("api_key", &"<redacted>")
            .field("secret", &"<redacted>")
            .field("passphrase", &"<redacted>")
            .finish()
    }
}

/// Polymarket API configuration (read + write actions via CLOB).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PolymarketConfig {
    #[serde(default = "default_polymarket_enabled")]
    pub enabled: bool,
    #[serde(default = "default_polymarket_gamma_base_url")]
    pub gamma_base_url: String,
    #[serde(default = "default_polymarket_clob_base_url")]
    pub clob_base_url: String,
    #[serde(default = "default_polymarket_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub eoa_address: Option<String>,
    #[serde(default = "default_polymarket_polygon_rpc_url")]
    pub polygon_rpc_url: String,
    #[serde(default = "default_polymarket_usdc_contract")]
    pub usdc_contract: String,
    #[serde(default = "default_polymarket_clob_exchange_contract")]
    pub clob_exchange_contract: String,
    /// Persisted L2 CLOB credentials (api_key, secret, passphrase) derived
    /// from the user's EOA via the L1 EIP-712 handshake against
    /// `/auth/api-key`.
    ///
    /// **Threat model — temporary plaintext.** Stored in the TOML config
    /// file in plaintext until #1900 lands the `SecretStore` encryption
    /// surface. Anything that reads the config (other tools, agents,
    /// disk-snapshot exfil) can exfiltrate the HMAC secret. Acceptable
    /// trade-off for a Beta feature that is off by default
    /// (`integrations.polymarket.enabled = false`) and explicitly
    /// opt-in. Migrate to SecretStore the moment #1900 merges — the in-
    /// memory cache (`PolymarketTool::cached_clob_credentials`) remains
    /// authoritative within a single process so the wire-level behaviour
    /// is unchanged on the migration.
    #[serde(default)]
    pub derived_clob_credentials: Option<PolymarketClobCredentials>,
}

impl Default for PolymarketConfig {
    fn default() -> Self {
        Self {
            enabled: default_polymarket_enabled(),
            gamma_base_url: default_polymarket_gamma_base_url(),
            clob_base_url: default_polymarket_clob_base_url(),
            timeout_secs: default_polymarket_timeout_secs(),
            eoa_address: None,
            polygon_rpc_url: default_polymarket_polygon_rpc_url(),
            usdc_contract: default_polymarket_usdc_contract(),
            clob_exchange_contract: default_polymarket_clob_exchange_contract(),
            derived_clob_credentials: None,
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
    /// Apify actor execution and scraper integration.
    #[serde(default)]
    pub apify: IntegrationToggle,

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

    /// Polymarket browse + trading APIs (Gamma + CLOB).
    #[serde(default)]
    pub polymarket: PolymarketConfig,
}

#[cfg(test)]
mod integration_toggle_tests {
    use super::*;

    #[test]
    fn managed_mode_active_when_enabled_without_key() {
        let toggle = IntegrationToggle {
            enabled: true,
            mode: INTEGRATION_MODE_MANAGED.into(),
            api_key: None,
        };
        assert!(toggle.is_active());
    }

    #[test]
    fn managed_mode_inactive_when_disabled() {
        let toggle = IntegrationToggle {
            enabled: false,
            mode: INTEGRATION_MODE_MANAGED.into(),
            api_key: Some("ignored".into()),
        };
        assert!(!toggle.is_active());
    }

    #[test]
    fn byo_mode_requires_non_empty_key() {
        let mut toggle = IntegrationToggle {
            enabled: true,
            mode: INTEGRATION_MODE_BYO.into(),
            api_key: None,
        };
        assert!(!toggle.is_active(), "missing key");

        toggle.api_key = Some("   ".into());
        assert!(!toggle.is_active(), "whitespace key");

        toggle.api_key = Some("real-key".into());
        assert!(toggle.is_active());
    }

    #[test]
    fn byo_mode_inactive_when_disabled_even_with_key() {
        let toggle = IntegrationToggle {
            enabled: false,
            mode: INTEGRATION_MODE_BYO.into(),
            api_key: Some("real-key".into()),
        };
        assert!(!toggle.is_active());
    }

    #[test]
    fn default_is_managed_and_active() {
        let toggle = IntegrationToggle::default();
        assert_eq!(toggle.mode, INTEGRATION_MODE_MANAGED);
        assert!(toggle.api_key.is_none());
        assert!(toggle.is_active());
    }
}
