//! Search settings, legacy engine migration, and separate Seltz/SearXNG options.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

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

// ── Search providers ────────────────────────────────────────────────
//
// Several providers can be active at once. Each provider has a route —
// `managed` (TinyHumans backend, billed to the signed-in session or API key)
// or `direct` (the user's own key). The agent does not see providers; it sees
// one tool per capability role (`search`, `answer`, `contents`), and each role
// is served by the first usable provider in its ordered list. Provider
// execution lives in the TinySearch module; this file only owns the settings
// and the one-time migration from the single-engine format.

/// Settings format written by this build, migrated on load:
/// - absent/0–1: the legacy single-engine fields → providers, routes, roles;
/// - 2: `presentation = "all_tools"` → `roles`. The v2 migration used to force
///   `all_tools` on every upgraded file, which hides the routed
///   `web_*_tool`s that agent tool scopes name, so those agents lost web
///   search. v3 moves them back to the routed tools once; a later explicit
///   choice is saved as v3 and kept. v3 also makes TinyFish own-key: the
///   backend never proxied it, so a managed TinyFish is turned off unless a
///   key is available.
pub const SEARCH_SCHEMA_VERSION: u32 = 3;

/// First format with providers, routes and roles.
pub(crate) const SEARCH_SCHEMA_PROVIDERS: u32 = 2;

pub const SEARCH_ROLE_SEARCH: &str = "search";
pub const SEARCH_ROLE_ANSWER: &str = "answer";
pub const SEARCH_ROLE_CONTENTS: &str = "contents";
/// Capability roles, in display order.
pub const SEARCH_ROLES: &[&str] = &[SEARCH_ROLE_SEARCH, SEARCH_ROLE_ANSWER, SEARCH_ROLE_CONTENTS];

/// Providers the host can configure — the TinySearch catalog.
pub const SEARCH_PROVIDERS: &[&str] = tinysearch_bus::PROVIDERS;

/// Providers that can be reached through the managed TinyHumans backend.
pub const MANAGED_SEARCH_PROVIDERS: &[&str] = tinysearch_bus::BACKEND_PROVIDERS;

// Legacy single-engine ids, still accepted by `config.update_search_settings`
// for older clients and by the `SEARCH_ENGINE` env var.
pub const SEARCH_ENGINE_DISABLED: &str = "disabled";
pub const SEARCH_ENGINE_MANAGED: &str = "managed";
pub const SEARCH_ENGINE_BRAVE: &str = "brave";
pub const SEARCH_ENGINE_QUERIT: &str = "querit";
pub const SEARCH_ENGINE_EXA: &str = "exa";
pub const SEARCH_ENGINE_TAVILY: &str = "tavily";
pub const SEARCH_ENGINE_PARALLEL: &str = "parallel";

fn default_search_max_results() -> usize {
    5
}

fn default_search_timeout_secs() -> u64 {
    15
}

/// Credentials for a BYO search provider. Considered configured iff the
/// trimmed value is non-empty.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SearchEngineCredentials {
    #[serde(default)]
    pub api_key: Option<String>,
}

impl SearchEngineCredentials {
    pub fn has_key(&self) -> bool {
        self.key().is_some()
    }

    pub fn key(&self) -> Option<&str> {
        self.api_key.as_deref().and_then(|s| {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        })
    }
}

/// How calls to a provider are routed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchRoute {
    /// Through the TinyHumans backend with the session credential.
    Managed,
    /// Straight to the provider with the user's own key.
    #[default]
    Direct,
}

impl SearchRoute {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Managed => "managed",
            Self::Direct => "direct",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "managed" | "backend" => Some(Self::Managed),
            "direct" => Some(Self::Direct),
            _ => None,
        }
    }
}

/// How search tools are presented to the agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchPresentation {
    /// One tool per capability role (the default).
    #[default]
    Roles,
    /// Every usable provider's own tools.
    AllTools,
    /// One `search` router tool with a provider argument.
    Router,
    /// Only the tools of `presentation_provider`.
    OneProvider,
}

impl SearchPresentation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Roles => "roles",
            Self::AllTools => "all_tools",
            Self::Router => "router",
            Self::OneProvider => "one_provider",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "roles" => Some(Self::Roles),
            "all_tools" => Some(Self::AllTools),
            "router" => Some(Self::Router),
            "one_provider" => Some(Self::OneProvider),
            _ => None,
        }
    }
}

/// Per-provider selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SearchProviderSettings {
    /// Whether the user turned this provider on.
    pub enabled: bool,
    /// Managed or direct.
    pub route: SearchRoute,
}

impl Default for SearchProviderSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            route: SearchRoute::Direct,
        }
    }
}

impl SearchProviderSettings {
    pub fn managed() -> Self {
        Self {
            enabled: true,
            route: SearchRoute::Managed,
        }
    }

    pub fn direct() -> Self {
        Self {
            enabled: true,
            route: SearchRoute::Direct,
        }
    }
}

/// Search configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SearchConfig {
    /// Settings format; see [`SEARCH_SCHEMA_VERSION`]. Absent in legacy files.
    #[serde(default)]
    pub schema_version: u32,

    /// Global switch. `None` only in legacy files before migration.
    #[serde(default)]
    pub enabled: Option<bool>,

    /// Provider selection and routes, keyed by [`SEARCH_PROVIDERS`] name.
    #[serde(default)]
    pub providers: BTreeMap<String, SearchProviderSettings>,

    /// Ordered provider list per role (keys from [`SEARCH_ROLES`]). A missing
    /// or empty role uses TinySearch's default order.
    #[serde(default)]
    pub roles: BTreeMap<String, Vec<String>>,

    /// Tool presentation.
    #[serde(default)]
    pub presentation: SearchPresentation,

    /// Provider for `one_provider`, or the router default.
    #[serde(default)]
    pub presentation_provider: Option<String>,

    /// Max results per query (1–20, default 5).
    #[serde(default = "default_search_max_results")]
    pub max_results: usize,

    /// Per-request timeout in seconds (default 15).
    #[serde(default = "default_search_timeout_secs")]
    pub timeout_secs: u64,

    /// Brave Search key (direct route).
    #[serde(default)]
    pub brave: SearchEngineCredentials,
    /// Querit key (direct route).
    #[serde(default)]
    pub querit: SearchEngineCredentials,
    /// Exa key (direct route). Managed Exa needs no key.
    #[serde(default)]
    pub exa: SearchEngineCredentials,
    /// Tavily key (direct route).
    #[serde(default)]
    pub tavily: SearchEngineCredentials,
    /// Gemini API key (direct route, and required for deep research).
    /// Managed Gemini needs no key.
    #[serde(default)]
    pub gemini: SearchEngineCredentials,
    /// Parallel key. Parallel is bring-your-own-key only: there is no managed
    /// Parallel route.
    #[serde(default)]
    pub parallel: SearchEngineCredentials,
    /// TinyFish key. TinyFish is bring-your-own-key only: the TinyHumans
    /// backend does not proxy it.
    #[serde(default)]
    pub tinyfish: SearchEngineCredentials,
    /// Keenable key (direct route). Optional: without one Keenable serves its
    /// keyless public endpoints, rate-limited per IP; a key raises the limits.
    #[serde(default)]
    pub keenable: SearchEngineCredentials,

    // ── Legacy single-engine fields: read for migration, never written ──
    #[serde(default, skip_serializing)]
    #[schemars(skip)]
    pub engine: Option<String>,
    #[serde(default, skip_serializing)]
    #[schemars(skip)]
    pub enabled_providers: Option<BTreeSet<String>>,
    #[serde(default, skip_serializing)]
    #[schemars(skip)]
    pub parallel_route: Option<String>,
    #[serde(default, skip_serializing)]
    #[schemars(skip)]
    pub gemini_route: Option<String>,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            schema_version: SEARCH_SCHEMA_VERSION,
            enabled: Some(true),
            providers: default_providers(),
            roles: BTreeMap::new(),
            presentation: SearchPresentation::Roles,
            presentation_provider: None,
            max_results: default_search_max_results(),
            timeout_secs: default_search_timeout_secs(),
            brave: SearchEngineCredentials::default(),
            querit: SearchEngineCredentials::default(),
            exa: SearchEngineCredentials::default(),
            tavily: SearchEngineCredentials::default(),
            gemini: SearchEngineCredentials::default(),
            engine: None,
            enabled_providers: None,
            parallel_route: None,
            gemini_route: None,
            parallel: SearchEngineCredentials::default(),
            tinyfish: SearchEngineCredentials::default(),
            keenable: SearchEngineCredentials::default(),
        }
    }
}

/// Fresh installs: managed Exa for ranked search and contents, managed Gemini
/// for grounded answers. Both need a signed-in session to become usable.
fn default_providers() -> BTreeMap<String, SearchProviderSettings> {
    BTreeMap::from([
        ("exa".to_string(), SearchProviderSettings::managed()),
        ("gemini".to_string(), SearchProviderSettings::managed()),
    ])
}

/// Legacy inputs that lived outside `[search]`.
#[derive(Debug, Clone, Default)]
pub struct LegacySearchInputs {
    /// `integrations.tinyfish.is_active()`.
    pub tinyfish_active: bool,
    /// `seltz.enabled` with a key.
    pub seltz_active: bool,
    /// `searxng.enabled`.
    pub searxng_active: bool,
    /// The key the legacy `integrations.tinyfish` toggle held, if any. TinyFish
    /// is own-key only now; this seeds `search.tinyfish` when it has none.
    pub tinyfish_api_key: Option<String>,
}

impl SearchConfig {
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    /// Stored key for a direct-route provider, if any. Seltz and SearXNG keep
    /// their options in their own sections and are resolved by the caller.
    pub fn credentials(&self, provider: &str) -> Option<&SearchEngineCredentials> {
        match provider {
            "brave" => Some(&self.brave),
            "querit" => Some(&self.querit),
            "exa" => Some(&self.exa),
            "tavily" => Some(&self.tavily),
            "gemini" | "gemini_deep_research" => Some(&self.gemini),
            "parallel" => Some(&self.parallel),
            "tinyfish" => Some(&self.tinyfish),
            "keenable" => Some(&self.keenable),
            _ => None,
        }
    }

    pub fn credentials_mut(&mut self, provider: &str) -> Option<&mut SearchEngineCredentials> {
        match provider {
            "brave" => Some(&mut self.brave),
            "querit" => Some(&mut self.querit),
            "exa" => Some(&mut self.exa),
            "tavily" => Some(&mut self.tavily),
            "gemini" | "gemini_deep_research" => Some(&mut self.gemini),
            "parallel" => Some(&mut self.parallel),
            "tinyfish" => Some(&mut self.tinyfish),
            "keenable" => Some(&mut self.keenable),
            _ => None,
        }
    }

    /// Enabled providers (ignores the global switch and usability).
    pub fn enabled_provider_names(&self) -> BTreeSet<String> {
        self.providers
            .iter()
            .filter(|(_, settings)| settings.enabled)
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Route for `provider`; providers that cannot be managed are always direct.
    pub fn route(&self, provider: &str) -> SearchRoute {
        if !MANAGED_SEARCH_PROVIDERS.contains(&provider) {
            return SearchRoute::Direct;
        }
        self.providers
            .get(provider)
            .map(|settings| settings.route)
            .unwrap_or(SearchRoute::Direct)
    }
}

#[path = "search_migrate.rs"]
mod migrate;

#[cfg(test)]
#[path = "search_search_config_tests_tests.rs"]
mod search_config_tests;
