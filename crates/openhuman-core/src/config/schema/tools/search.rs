//! Search settings, legacy engine migration, and separate Seltz/SearXNG options.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

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

// ── Search engines ──────────────────────────────────────────────────
//
// Unified search-engine selector. Only one engine is active at a time
// (mirrors the LLM-provider API-key flow). The active engine governs
// which tools are registered: `disabled` → no search tools; `managed` →
// backend-proxied `web_search`; `parallel` → direct Parallel API tools
// (search/extract/chat/research/enrich/dataset); `brave` → direct Brave Search
// tools (web/news/images/videos); `querit` → direct Querit web search;
// `exa` → direct Exa neural search (search / find similar / contents);
// `tavily` → direct Tavily Search + Extract (web / news / finance).

pub const SEARCH_ENGINE_DISABLED: &str = "disabled";
pub const SEARCH_ENGINE_MANAGED: &str = "managed";
pub const SEARCH_ENGINE_PARALLEL: &str = "parallel";
pub const SEARCH_ENGINE_BRAVE: &str = "brave";
pub const SEARCH_ENGINE_QUERIT: &str = "querit";
pub const SEARCH_ENGINE_EXA: &str = "exa";
pub const SEARCH_ENGINE_TAVILY: &str = "tavily";

fn default_search_engine() -> String {
    SEARCH_ENGINE_MANAGED.into()
}

fn default_search_max_results() -> usize {
    5
}

fn default_search_timeout_secs() -> u64 {
    15
}

/// Credentials for a BYO search engine. Mirrors the LLM provider API-
/// key shape — a simple `Option<String>` that is considered configured
/// iff the trimmed value is non-empty.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SearchEngineCredentials {
    #[serde(default)]
    pub api_key: Option<String>,
}

impl SearchEngineCredentials {
    pub fn has_key(&self) -> bool {
        self.api_key
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
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

/// Search configuration. New settings select a provider set; an omitted set
/// derives available providers from saved keys, the backend credential, and
/// the separate TinyFish toggle. The legacy engine remains for migration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SearchConfig {
    /// Active search engine. One of [`SEARCH_ENGINE_DISABLED`],
    /// [`SEARCH_ENGINE_MANAGED`], [`SEARCH_ENGINE_PARALLEL`],
    /// [`SEARCH_ENGINE_BRAVE`], [`SEARCH_ENGINE_QUERIT`],
    /// [`SEARCH_ENGINE_EXA`], or [`SEARCH_ENGINE_TAVILY`]. Unknown values fall
    /// back to managed at registration time.
    #[serde(default = "default_search_engine")]
    pub engine: String,

    /// Explicit global switch. Missing in older config files, where `engine`
    /// still decides whether search was disabled.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Explicit provider selection. Missing migrates from saved credentials
    /// and the legacy TinyFish integration toggle. When present, this set is
    /// authoritative, including for TinyFish.
    /// Managed and direct Parallel share one module route; explicit Parallel
    /// selection takes precedence when both are selected.
    #[serde(default)]
    pub enabled_providers: Option<BTreeSet<String>>,
    /// `all_tools`, `router`, or `one_provider`.
    #[serde(default = "default_search_presentation")]
    pub presentation: String,
    /// Provider used by `one_provider`, or router default.
    #[serde(default)]
    pub presentation_provider: Option<String>,
    /// `direct` or `backend` for Parallel.
    #[serde(default = "default_direct_route")]
    pub parallel_route: String,
    /// `direct` or `backend` for Gemini.
    #[serde(default = "default_direct_route")]
    pub gemini_route: String,

    /// Max results per query (1–20, default 5).
    #[serde(default = "default_search_max_results")]
    pub max_results: usize,

    /// Per-request timeout in seconds (default 15).
    #[serde(default = "default_search_timeout_secs")]
    pub timeout_secs: u64,

    /// Parallel API credentials (used when `engine = "parallel"`).
    #[serde(default)]
    pub parallel: SearchEngineCredentials,

    /// Brave Search credentials (used when `engine = "brave"`).
    #[serde(default)]
    pub brave: SearchEngineCredentials,

    /// Querit credentials (used when `engine = "querit"`).
    #[serde(default)]
    pub querit: SearchEngineCredentials,

    /// Exa credentials (used when `engine = "exa"`). BYOK: search calls go
    /// straight to `https://api.exa.ai`, never through the managed backend.
    #[serde(default)]
    pub exa: SearchEngineCredentials,

    /// Tavily credentials (used when `engine = "tavily"`). BYOK: search and
    /// extract calls go straight to `https://api.tavily.com`, never through
    /// the managed backend.
    #[serde(default)]
    pub tavily: SearchEngineCredentials,

    /// Gemini direct API credential.
    #[serde(default)]
    pub gemini: SearchEngineCredentials,
}

fn default_search_presentation() -> String {
    "all_tools".into()
}
fn default_direct_route() -> String {
    "direct".into()
}

pub const SEARCH_PROVIDERS: &[&str] = &[
    "managed", "parallel", "brave", "querit", "exa", "tavily", "gemini", "tinyfish", "seltz",
    "searxng",
];

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            engine: default_search_engine(),
            enabled: None,
            enabled_providers: None,
            presentation: default_search_presentation(),
            presentation_provider: None,
            parallel_route: default_direct_route(),
            gemini_route: default_direct_route(),
            max_results: default_search_max_results(),
            timeout_secs: default_search_timeout_secs(),
            parallel: SearchEngineCredentials::default(),
            brave: SearchEngineCredentials::default(),
            querit: SearchEngineCredentials::default(),
            exa: SearchEngineCredentials::default(),
            tavily: SearchEngineCredentials::default(),
            gemini: SearchEngineCredentials::default(),
        }
    }
}

/// Normalized search-engine enum used at tool-registration time. Falls
/// back to [`SearchEngine::Managed`] for unknown strings and for BYO
/// engines that have no API key configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchEngine {
    Disabled,
    Managed,
    Parallel,
    Brave,
    Querit,
    Exa,
    Tavily,
}

impl SearchConfig {
    pub fn is_enabled(&self) -> bool {
        self.enabled
            .unwrap_or(self.engine.trim() != SEARCH_ENGINE_DISABLED)
    }

    /// Resolve provider selections. `None` is the old single-engine format.
    pub fn providers(
        &self,
        managed_available: bool,
        tinyfish_enabled: bool,
        seltz_enabled: bool,
        searxng_enabled: bool,
    ) -> BTreeSet<String> {
        if !self.is_enabled() {
            return BTreeSet::new();
        }
        if let Some(selected) = &self.enabled_providers {
            return selected.clone();
        }
        let mut providers = BTreeSet::new();
        if managed_available {
            providers.insert("managed".into());
        }
        for (name, credentials) in [
            ("parallel", &self.parallel),
            ("brave", &self.brave),
            ("querit", &self.querit),
            ("exa", &self.exa),
            ("tavily", &self.tavily),
            ("gemini", &self.gemini),
        ] {
            if credentials.has_key() {
                providers.insert(name.into());
            }
        }
        if tinyfish_enabled {
            providers.insert("tinyfish".into());
        }
        if seltz_enabled {
            providers.insert("seltz".into());
        }
        if searxng_enabled {
            providers.insert("searxng".into());
        }
        providers
    }

    /// Resolve the *effective* engine after gating on API-key
    /// availability. A BYO engine without a key silently falls back to
    /// managed so the agent never ends up with zero search tools — the
    /// UI surfaces the misconfiguration separately.
    pub fn effective_engine(&self) -> SearchEngine {
        match self.engine.trim().to_ascii_lowercase().as_str() {
            SEARCH_ENGINE_DISABLED => SearchEngine::Disabled,
            SEARCH_ENGINE_PARALLEL if self.parallel.has_key() => SearchEngine::Parallel,
            SEARCH_ENGINE_BRAVE if self.brave.has_key() => SearchEngine::Brave,
            SEARCH_ENGINE_QUERIT if self.querit.has_key() => SearchEngine::Querit,
            SEARCH_ENGINE_EXA if self.exa.has_key() => SearchEngine::Exa,
            SEARCH_ENGINE_TAVILY if self.tavily.has_key() => SearchEngine::Tavily,
            _ => SearchEngine::Managed,
        }
    }

    pub fn requested_engine_str(&self) -> &str {
        let trimmed = self.engine.trim();
        if trimmed.is_empty() {
            SEARCH_ENGINE_MANAGED
        } else {
            trimmed
        }
    }
}

#[cfg(test)]
#[path = "search_search_config_tests_tests.rs"]
mod search_config_tests;
