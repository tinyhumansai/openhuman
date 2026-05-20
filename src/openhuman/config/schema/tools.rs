//! Tool-related config: browser, HTTP, web search, composio, secrets, multimodal.

use super::defaults;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MultimodalConfig {
    #[serde(default = "default_multimodal_max_images")]
    pub max_images: usize,
    #[serde(default = "default_multimodal_max_image_size_mb")]
    pub max_image_size_mb: usize,
    #[serde(default)]
    pub allow_remote_fetch: bool,
}

fn default_multimodal_max_images() -> usize {
    4
}

fn default_multimodal_max_image_size_mb() -> usize {
    8
}

impl MultimodalConfig {
    /// Clamp configured values to safe runtime bounds.
    pub fn effective_limits(&self) -> (usize, usize) {
        let max_images = self.max_images.clamp(1, 16);
        let max_image_size_mb = self.max_image_size_mb.clamp(1, 20);
        (max_images, max_image_size_mb)
    }

    /// Clamp image count to the configured maximum.
    pub fn clamp_image_count(&self, count: usize) -> usize {
        count.min(self.max_images)
    }
}

impl Default for MultimodalConfig {
    fn default() -> Self {
        Self {
            max_images: default_multimodal_max_images(),
            max_image_size_mb: default_multimodal_max_image_size_mb(),
            allow_remote_fetch: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BrowserComputerUseConfig {
    #[serde(default = "default_browser_computer_use_endpoint")]
    pub endpoint: String,
    #[serde(default = "default_browser_computer_use_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub allow_remote_endpoint: bool,
    #[serde(default)]
    pub window_allowlist: Vec<String>,
    #[serde(default)]
    pub max_coordinate_x: Option<i64>,
    #[serde(default)]
    pub max_coordinate_y: Option<i64>,
}

fn default_browser_computer_use_endpoint() -> String {
    "http://127.0.0.1:8787/v1/actions".into()
}

fn default_browser_computer_use_timeout_ms() -> u64 {
    15_000
}

impl Default for BrowserComputerUseConfig {
    fn default() -> Self {
        Self {
            endpoint: default_browser_computer_use_endpoint(),
            timeout_ms: default_browser_computer_use_timeout_ms(),
            allow_remote_endpoint: false,
            window_allowlist: Vec::new(),
            max_coordinate_x: None,
            max_coordinate_y: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BrowserConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default)]
    pub session_name: Option<String>,
    #[serde(default = "default_browser_backend")]
    pub backend: String,
    #[serde(default = "default_true")]
    pub native_headless: bool,
    #[serde(default = "default_browser_webdriver_url")]
    pub native_webdriver_url: String,
    #[serde(default)]
    pub native_chrome_path: Option<String>,
    #[serde(default)]
    pub computer_use: BrowserComputerUseConfig,
}

fn default_true() -> bool {
    defaults::default_true()
}

fn default_browser_backend() -> String {
    "agent_browser".into()
}

fn default_browser_webdriver_url() -> String {
    "http://127.0.0.1:9515".into()
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_domains: Vec::new(),
            session_name: None,
            backend: default_browser_backend(),
            native_headless: default_true(),
            native_webdriver_url: default_browser_webdriver_url(),
            native_chrome_path: None,
            computer_use: BrowserComputerUseConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(default)]
pub struct HttpRequestConfig {
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default = "default_http_max_response_size")]
    pub max_response_size: usize,
    #[serde(default = "default_http_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_http_max_response_size() -> usize {
    1_000_000
}

fn default_http_timeout_secs() -> u64 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CurlConfig {
    /// Subdirectory under `workspace_dir` where downloads land. Inputs
    /// are resolved relative to this root; absolute paths and `..`
    /// segments are rejected.
    #[serde(default = "default_curl_dest_subdir")]
    pub dest_subdir: String,
    /// Hard byte ceiling per download. Streaming aborts and the
    /// partial file is removed if exceeded.
    #[serde(default = "default_curl_max_download_bytes")]
    pub max_download_bytes: u64,
    /// Per-request timeout in seconds.
    #[serde(default = "default_curl_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_curl_dest_subdir() -> String {
    "downloads".into()
}

fn default_curl_max_download_bytes() -> u64 {
    50 * 1024 * 1024
}

fn default_curl_timeout_secs() -> u64 {
    120
}

impl Default for CurlConfig {
    fn default() -> Self {
        Self {
            dest_subdir: default_curl_dest_subdir(),
            max_download_bytes: default_curl_max_download_bytes(),
            timeout_secs: default_curl_timeout_secs(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct GitbooksConfig {
    /// When `true`, register `gitbooks_search` and `gitbooks_get_page`.
    #[serde(default = "defaults::default_true")]
    pub enabled: bool,
    /// MCP endpoint URL for the OpenHuman GitBook docs.
    #[serde(default = "default_gitbooks_endpoint")]
    pub endpoint: String,
    /// Per-request timeout in seconds.
    #[serde(default = "default_gitbooks_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_gitbooks_endpoint() -> String {
    "https://tinyhumans.gitbook.io/openhuman/~gitbook/mcp".into()
}

fn default_gitbooks_timeout_secs() -> u64 {
    30
}

impl Default for GitbooksConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::default_true(),
            endpoint: default_gitbooks_endpoint(),
            timeout_secs: default_gitbooks_timeout_secs(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct McpServerConfig {
    /// Stable server slug used by the agent-facing bridge tools.
    #[serde(default)]
    pub name: String,
    /// MCP endpoint URL. Current implementation supports stateless
    /// Streamable HTTP / JSON responses.
    #[serde(default)]
    pub endpoint: String,
    /// Optional stdio command for local MCP servers. When set, the
    /// client launches this command as a subprocess and speaks newline-
    /// delimited JSON-RPC over stdin/stdout per the MCP stdio transport.
    #[serde(default)]
    pub command: String,
    /// Command-line arguments for stdio MCP servers.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables for stdio MCP servers. MCP stdio auth
    /// is typically passed this way.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Optional working directory for stdio MCP servers.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Optional human-readable description shown in bridge tool output.
    #[serde(default)]
    pub description: Option<String>,
    /// Whether this server should be exposed to the MCP bridge tools.
    #[serde(default = "defaults::default_true")]
    pub enabled: bool,
    /// Exact remote tool names this server may expose through the generic
    /// MCP bridge. Empty means all remote tools are allowed unless they
    /// appear in `disallowed_tools`.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Exact remote tool names that should always be hidden and blocked.
    /// This denylist takes precedence over `allowed_tools`.
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    /// Per-request timeout in seconds.
    #[serde(default = "default_mcp_timeout_secs")]
    pub timeout_secs: u64,
    /// Optional auth strategy applied to outbound requests for this
    /// server. Useful for API-key and pre-provisioned bearer-token
    /// flows; interactive OAuth discovery is handled by the client
    /// transport separately when a server returns an auth challenge.
    #[serde(default)]
    pub auth: McpAuthConfig,
}

fn default_mcp_timeout_secs() -> u64 {
    30
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            endpoint: String::new(),
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            cwd: None,
            description: None,
            enabled: defaults::default_true(),
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            timeout_secs: default_mcp_timeout_secs(),
            auth: McpAuthConfig::None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpAuthConfig {
    None,
    BearerToken { token: String },
    Basic { username: String, password: String },
    Header { name: String, value: String },
    QueryParam { name: String, value: String },
}

impl Default for McpAuthConfig {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct McpClientIdentityConfig {
    /// Client name sent during `initialize.clientInfo.name`.
    #[serde(default = "default_mcp_client_name")]
    pub name: String,
    /// Client title sent during `initialize.clientInfo.title`.
    #[serde(default = "default_mcp_client_title")]
    pub title: String,
    /// Client version sent during `initialize.clientInfo.version`.
    #[serde(default = "default_mcp_client_version")]
    pub version: String,
}

fn default_mcp_client_name() -> String {
    "openhuman-core".into()
}

fn default_mcp_client_title() -> String {
    "OpenHuman Core MCP Client".into()
}

fn default_mcp_client_version() -> String {
    env!("CARGO_PKG_VERSION").into()
}

impl Default for McpClientIdentityConfig {
    fn default() -> Self {
        Self {
            name: default_mcp_client_name(),
            title: default_mcp_client_title(),
            version: default_mcp_client_version(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct McpClientConfig {
    /// When `true`, register the generic MCP bridge tools and expose
    /// configured remote MCP servers to the agent runtime.
    #[serde(default = "defaults::default_true")]
    pub enabled: bool,
    /// Named remote MCP servers accessible via `mcp_list_*` /
    /// `mcp_call_tool`.
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
    /// Identity block sent during initialize.
    #[serde(default)]
    pub client_identity: McpClientIdentityConfig,
}

impl Default for McpClientConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::default_true(),
            servers: Vec::new(),
            client_identity: McpClientIdentityConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SeltzConfig {
    /// When `true`, register `seltz_search` as an agent tool.
    #[serde(default)]
    pub enabled: bool,
    /// Seltz API key. Can also be set via `SELTZ_API_KEY` or
    /// `OPENHUMAN_SELTZ_API_KEY` env var.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Override the Seltz API base URL (default: `https://api.seltz.ai/v1`).
    #[serde(default)]
    pub api_url: Option<String>,
    /// Max results per query (1–20, default 10).
    #[serde(default = "default_seltz_max_results")]
    pub max_results: usize,
    /// Per-request timeout in seconds (default 15).
    #[serde(default = "default_seltz_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_seltz_max_results() -> usize {
    10
}

fn default_seltz_timeout_secs() -> u64 {
    15
}

impl Default for SeltzConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: None,
            api_url: None,
            max_results: default_seltz_max_results(),
            timeout_secs: default_seltz_timeout_secs(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SearxngConfig {
    /// When `true`, register `searxng_search` as an agent and MCP tool.
    #[serde(default)]
    pub enabled: bool,
    /// Base URL for the user's SearXNG instance.
    #[serde(default = "default_searxng_base_url")]
    pub base_url: String,
    /// Max results per query (1-50, default 10).
    #[serde(default = "default_searxng_max_results")]
    pub max_results: usize,
    /// Language code passed to SearXNG when a call omits `language`.
    #[serde(default = "default_searxng_language")]
    pub default_language: String,
    /// Per-request timeout in seconds (default 10).
    #[serde(default = "default_searxng_timeout_secs", alias = "timeout_seconds")]
    pub timeout_secs: u64,
}

fn default_searxng_base_url() -> String {
    "http://localhost:8080".into()
}

fn default_searxng_max_results() -> usize {
    10
}

fn default_searxng_language() -> String {
    "en".into()
}

fn default_searxng_timeout_secs() -> u64 {
    10
}

impl Default for SearxngConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: default_searxng_base_url(),
            max_results: default_searxng_max_results(),
            default_language: default_searxng_language(),
            timeout_secs: default_searxng_timeout_secs(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WebSearchConfig {
    #[serde(default = "default_web_search_max_results")]
    pub max_results: usize,
    #[serde(default = "default_web_search_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_web_search_max_results() -> usize {
    5
}

fn default_web_search_timeout_secs() -> u64 {
    15
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            max_results: default_web_search_max_results(),
            timeout_secs: default_web_search_timeout_secs(),
        }
    }
}

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
/// sink. See `tools/impl/network/composio.rs` for the underlying client.
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
    #[serde(default = "default_true")]
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
