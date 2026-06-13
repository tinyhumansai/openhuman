//! Search engine config: Seltz, SearXNG, web search, and unified search selector.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
// tools (web/news/images/videos); `querit` → direct Querit web search.

pub const SEARCH_ENGINE_DISABLED: &str = "disabled";
pub const SEARCH_ENGINE_MANAGED: &str = "managed";
pub const SEARCH_ENGINE_PARALLEL: &str = "parallel";
pub const SEARCH_ENGINE_BRAVE: &str = "brave";
pub const SEARCH_ENGINE_QUERIT: &str = "querit";

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

/// Unified search-engine configuration. Exactly one engine drives tool
/// registration at a time. `disabled` suppresses all search tools; `managed` is
/// the backend-proxied default and requires no key; `parallel`, `brave`, and
/// `querit` are BYO and require their own API key in the matching sub-block.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SearchConfig {
    /// Active search engine. One of [`SEARCH_ENGINE_DISABLED`],
    /// [`SEARCH_ENGINE_MANAGED`], [`SEARCH_ENGINE_PARALLEL`],
    /// [`SEARCH_ENGINE_BRAVE`], or [`SEARCH_ENGINE_QUERIT`]. Unknown values
    /// fall back to managed at registration time.
    #[serde(default = "default_search_engine")]
    pub engine: String,

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
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            engine: default_search_engine(),
            max_results: default_search_max_results(),
            timeout_secs: default_search_timeout_secs(),
            parallel: SearchEngineCredentials::default(),
            brave: SearchEngineCredentials::default(),
            querit: SearchEngineCredentials::default(),
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
}

impl SearchConfig {
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
mod search_config_tests {
    use super::*;
    use crate::openhuman::config::schema::tools::http::HttpRequestConfig;

    #[test]
    fn defaults_to_managed() {
        let cfg = SearchConfig::default();
        assert_eq!(cfg.effective_engine(), SearchEngine::Managed);
    }

    #[test]
    fn disabled_stays_disabled() {
        let cfg = SearchConfig {
            engine: SEARCH_ENGINE_DISABLED.into(),
            ..Default::default()
        };
        assert_eq!(cfg.effective_engine(), SearchEngine::Disabled);
    }

    #[test]
    fn parallel_requires_key() {
        let mut cfg = SearchConfig {
            engine: SEARCH_ENGINE_PARALLEL.into(),
            ..Default::default()
        };
        assert_eq!(cfg.effective_engine(), SearchEngine::Managed);
        cfg.parallel.api_key = Some("  ".into());
        assert_eq!(cfg.effective_engine(), SearchEngine::Managed);
        cfg.parallel.api_key = Some("real".into());
        assert_eq!(cfg.effective_engine(), SearchEngine::Parallel);
    }

    #[test]
    fn brave_requires_key() {
        let mut cfg = SearchConfig {
            engine: SEARCH_ENGINE_BRAVE.into(),
            ..Default::default()
        };
        assert_eq!(cfg.effective_engine(), SearchEngine::Managed);
        cfg.brave.api_key = Some("real".into());
        assert_eq!(cfg.effective_engine(), SearchEngine::Brave);
    }

    #[test]
    fn querit_requires_key() {
        let mut cfg = SearchConfig {
            engine: SEARCH_ENGINE_QUERIT.into(),
            ..Default::default()
        };
        assert_eq!(cfg.effective_engine(), SearchEngine::Managed);
        cfg.querit.api_key = Some("real".into());
        assert_eq!(cfg.effective_engine(), SearchEngine::Querit);
    }

    #[test]
    fn http_request_defaults_to_allow_all() {
        // Web research works out of the box: the default allowlist is the
        // wildcard. The SSRF guard (url_guard) still blocks local/private
        // hosts regardless, so this only opens public sites.
        let cfg = HttpRequestConfig::default();
        assert_eq!(cfg.allowed_domains, vec!["*".to_string()]);
        assert_eq!(cfg.max_response_size, 1_000_000);
        assert_eq!(cfg.timeout_secs, 30);
    }

    #[test]
    fn unknown_engine_falls_back_to_managed() {
        let cfg = SearchConfig {
            engine: "duckduckgo".into(),
            ..Default::default()
        };
        assert_eq!(cfg.effective_engine(), SearchEngine::Managed);
    }
}
