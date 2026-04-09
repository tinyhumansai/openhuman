//! Tool-related config: browser, HTTP, web search, composio, secrets, multimodal.

use super::defaults;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
pub struct BrowserComputerUseConfig {
    #[serde(default = "default_browser_computer_use_endpoint")]
    pub endpoint: String,
    #[serde(default)]
    pub api_key: Option<String>,
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
            api_key: None,
            timeout_ms: default_browser_computer_use_timeout_ms(),
            allow_remote_endpoint: false,
            window_allowlist: Vec::new(),
            max_coordinate_x: None,
            max_coordinate_y: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
pub struct HttpRequestConfig {
    #[serde(default)]
    pub enabled: bool,
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
pub struct WebSearchConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Search provider. Valid values: `duckduckgo` (default, free), `brave` (requires `brave_api_key`),
    /// `parallel` (requires `parallel_api_key`).
    #[serde(default = "default_web_search_provider")]
    pub provider: String,
    /// API key for the Brave Search API. Set via `OPENHUMAN_BRAVE_API_KEY` / `BRAVE_API_KEY`.
    #[serde(default)]
    pub brave_api_key: Option<String>,
    /// API key for the Parallel Search API (<https://docs.parallel.ai>).
    /// Set via `OPENHUMAN_PARALLEL_API_KEY` / `PARALLEL_API_KEY`.
    #[serde(default)]
    pub parallel_api_key: Option<String>,
    #[serde(default = "default_web_search_max_results")]
    pub max_results: usize,
    #[serde(default = "default_web_search_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_web_search_provider() -> String {
    "duckduckgo".into()
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
            enabled: true,
            provider: default_web_search_provider(),
            brave_api_key: None,
            parallel_api_key: None,
            max_results: default_web_search_max_results(),
            timeout_secs: default_web_search_timeout_secs(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ComposioConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_entity_id")]
    pub entity_id: String,
}

fn default_entity_id() -> String {
    "default".into()
}

impl Default for ComposioConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: None,
            entity_id: default_entity_id(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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

// ── Agent integration tools (backend-proxied) ───────────────────────

/// Per-integration on/off toggle.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IntegrationToggle {
    #[serde(default = "defaults::default_true")]
    pub enabled: bool,
}

impl Default for IntegrationToggle {
    fn default() -> Self {
        Self {
            enabled: defaults::default_true(),
        }
    }
}

/// Agent integration tools that proxy through the backend API.
///
/// When enabled, the agent gains access to tools like web search (Parallel),
/// location search (Google Places), and phone calls (Twilio). The backend
/// handles external API calls, billing, and rate limiting; the client only
/// forwards requests and displays results.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct IntegrationsConfig {
    /// Master switch — set to `true` to register integration tools.
    #[serde(default)]
    pub enabled: bool,

    /// Backend API base URL (e.g. "https://api.openhuman.ai").
    #[serde(default)]
    pub backend_url: Option<String>,

    /// JWT Bearer token for authenticating with the backend.
    #[serde(default)]
    pub auth_token: Option<String>,

    /// Twilio phone-call integration.
    #[serde(default)]
    pub twilio: IntegrationToggle,

    /// Google Places location search integration.
    #[serde(default)]
    pub google_places: IntegrationToggle,

    /// Parallel web search & content extraction integration.
    #[serde(default)]
    pub parallel: IntegrationToggle,
}
