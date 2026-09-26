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

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
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
    /// [`crate::security::credentials`] under the
    /// `composio-direct` provider slot. We only persist the mode here so
    /// the factory can pick the right client at construction time.
    #[serde(default = "default_composio_mode")]
    pub mode: String,

    /// **Deprecated for direct storage** — present so users that hand-edit
    /// `config.toml` can drop the key in here. The factory still prefers
    /// the keychain-backed value over this field. Default `None`.
    #[serde(default)]
    pub api_key: Option<String>,

    /// Gmail search query that scopes the background Gmail→memory sync to
    /// matching messages only (full Gmail search syntax — `label:brain`,
    /// `from:someone`, `newer_than:30d`; space-separated clauses AND).
    /// Empty (the default) = the whole inbox window. On-demand agent access
    /// to Gmail is unaffected — this gates only what auto-ingests into
    /// memory.
    #[serde(default)]
    pub gmail_sync_query: String,

    /// A direct-mode credential an embedder pinned for one agent. Never
    /// persisted. While set, Composio tools resolve against this config
    /// rather than reloading `config_path`, and its key wins over the
    /// shared credential store. Set it with [`ComposioConfig::pin_host_credential`].
    #[serde(skip)]
    pub host_credential: Option<ComposioHostCredential>,
}

impl ComposioConfig {
    /// Pin `credential` as this agent's Composio identity: direct mode,
    /// its key and entity, and no fallback to the shared credential store.
    pub fn pin_host_credential(&mut self, credential: ComposioHostCredential) {
        self.mode = COMPOSIO_MODE_DIRECT.into();
        self.entity_id = credential.entity_id.clone();
        self.host_credential = Some(credential);
    }
}

impl std::fmt::Debug for ComposioConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComposioConfig")
            .field("enabled", &self.enabled)
            .field("entity_id", &self.entity_id)
            .field("triage_disabled", &self.triage_disabled)
            .field("triage_disabled_toolkits", &self.triage_disabled_toolkits)
            .field("mode", &self.mode)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("gmail_sync_query", &self.gmail_sync_query)
            .field("host_credential", &self.host_credential)
            .finish()
    }
}

/// Composio v2/v3 API roots for a pinned direct-mode credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposioDirectBaseUrls {
    pub v2: String,
    pub v3: String,
}

/// A per-agent Composio direct-mode credential supplied by an embedder.
#[derive(Clone, PartialEq, Eq)]
pub struct ComposioHostCredential {
    api_key: String,
    entity_id: String,
    base_urls: Option<ComposioDirectBaseUrls>,
}

impl ComposioHostCredential {
    /// A direct-mode credential for `api_key`, on the `"default"` entity.
    pub fn direct(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into().trim().to_string(),
            entity_id: default_entity_id(),
            base_urls: None,
        }
    }

    /// The Composio entity (user id) this agent acts as.
    #[must_use]
    pub fn entity_id(mut self, entity_id: impl Into<String>) -> Self {
        let entity_id = entity_id.into();
        let trimmed = entity_id.trim();
        self.entity_id = if trimmed.is_empty() {
            default_entity_id()
        } else {
            trimmed.to_string()
        };
        self
    }

    /// Route this credential's calls to other Composio API roots. Must be
    /// HTTPS; loopback HTTP is accepted only in debug builds.
    #[must_use]
    pub fn base_urls(mut self, v2: impl Into<String>, v3: impl Into<String>) -> Self {
        self.base_urls = Some(ComposioDirectBaseUrls {
            v2: v2.into(),
            v3: v3.into(),
        });
        self
    }

    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    pub fn entity(&self) -> &str {
        &self.entity_id
    }

    pub fn direct_base_urls(&self) -> Option<&ComposioDirectBaseUrls> {
        self.base_urls.as_ref()
    }
}

impl std::fmt::Debug for ComposioHostCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComposioHostCredential")
            .field("api_key", &"<redacted>")
            .field("entity_id", &self.entity_id)
            .field("base_urls", &self.base_urls)
            .finish()
    }
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
            gmail_sync_query: String::new(),
            host_credential: None,
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

#[cfg(test)]
#[path = "integrations_host_credential_tests.rs"]
mod host_credential_tests;
