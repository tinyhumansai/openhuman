//! Env overrides for web search: SearXNG, Seltz, Tavily, and the search engine selection.

use crate::config::schema::load::env::parse_env_bool;
use crate::config::schema::load::env::EnvLookup;
use crate::config::schema::Config;

impl Config {
    pub(super) fn apply_search_env<E: EnvLookup + ?Sized>(&mut self, env: &E) {
        if let Some(key) = env.get_any(&["OPENHUMAN_SELTZ_API_KEY", "SELTZ_API_KEY"]) {
            if !key.is_empty() {
                self.seltz.api_key = Some(key);
                self.seltz.enabled = true;
            }
        }
        if let Some(url) = env.get_any(&["OPENHUMAN_SELTZ_API_URL", "SELTZ_API_URL"]) {
            if !url.is_empty() {
                self.seltz.api_url = Some(url);
            }
        }
        if let Some(max) = env.get_any(&["OPENHUMAN_SELTZ_MAX_RESULTS", "SELTZ_MAX_RESULTS"]) {
            if let Ok(n) = max.parse::<usize>() {
                if (1..=20).contains(&n) {
                    self.seltz.max_results = n;
                }
            }
        }

        if let Some(flag) = env.get_any(&["OPENHUMAN_SEARXNG_ENABLED", "SEARXNG_ENABLED"]) {
            if let Some(enabled) = parse_env_bool("OPENHUMAN_SEARXNG_ENABLED", &flag) {
                self.searxng.enabled = enabled;
            }
        }
        if let Some(url) = env.get_any(&["OPENHUMAN_SEARXNG_BASE_URL", "SEARXNG_BASE_URL"]) {
            let url = url.trim();
            if !url.is_empty() {
                self.searxng.base_url = url.to_string();
            }
        }
        if let Some(max) = env.get_any(&["OPENHUMAN_SEARXNG_MAX_RESULTS", "SEARXNG_MAX_RESULTS"]) {
            if let Ok(n) = max.parse::<usize>() {
                if (1..=50).contains(&n) {
                    self.searxng.max_results = n;
                }
            }
        }
        if let Some(language) = env.get_any(&[
            "OPENHUMAN_SEARXNG_DEFAULT_LANGUAGE",
            "SEARXNG_DEFAULT_LANGUAGE",
        ]) {
            let language = language.trim();
            if !language.is_empty() {
                self.searxng.default_language = language.to_string();
            }
        }
        if let Some(timeout_secs) = env.get_any(&[
            "OPENHUMAN_SEARXNG_TIMEOUT_SECS",
            "OPENHUMAN_SEARXNG_TIMEOUT_SECONDS",
            "SEARXNG_TIMEOUT_SECS",
            "SEARXNG_TIMEOUT_SECONDS",
        ]) {
            if let Ok(timeout_secs) = timeout_secs.parse::<u64>() {
                if timeout_secs > 0 {
                    self.searxng.timeout_secs = timeout_secs;
                }
            }
        }

        if let Some(flag) = env.get_any(&["OPENHUMAN_SEARCH_ENABLED"]) {
            if let Some(enabled) = parse_env_bool("OPENHUMAN_SEARCH_ENABLED", &flag) {
                self.search.enabled = Some(enabled);
            }
        }
        if let Some(names) = env.get_any(&["OPENHUMAN_SEARCH_PROVIDERS"]) {
            let selected: std::collections::BTreeSet<String> = names
                .split(',')
                .map(|name| name.trim().to_ascii_lowercase())
                .filter(|name| crate::config::schema::SEARCH_PROVIDERS.contains(&name.as_str()))
                .collect();
            self.search.enabled_providers = Some(selected);
        }
        if let Some(mode) = env.get_any(&["OPENHUMAN_SEARCH_PRESENTATION"]) {
            if ["all_tools", "router", "one_provider"].contains(&mode.as_str()) {
                self.search.presentation = mode;
            }
        }
        for (name, route) in [
            ("OPENHUMAN_PARALLEL_ROUTE", &mut self.search.parallel_route),
            ("OPENHUMAN_GEMINI_ROUTE", &mut self.search.gemini_route),
        ] {
            if let Some(value) = env.get_any(&[name]) {
                if value == "direct" || value == "backend" {
                    *route = value;
                }
            }
        }
        if let Some(key) = env.get_any(&["OPENHUMAN_GEMINI_API_KEY", "GEMINI_API_KEY"]) {
            if !key.trim().is_empty() {
                self.search.gemini.api_key = Some(key);
            }
        }
        if let Some(engine) = env.get_any(&["OPENHUMAN_SEARCH_ENGINE", "SEARCH_ENGINE"]) {
            let engine = engine.trim().to_ascii_lowercase();
            if !engine.is_empty() {
                self.search.engine = engine;
            }
        }
        if let Some(key) = env.get_any(&["OPENHUMAN_PARALLEL_API_KEY", "PARALLEL_API_KEY"]) {
            if !key.trim().is_empty() {
                self.search.parallel.api_key = Some(key);
            }
        }
        if let Some(key) = env.get_any(&["OPENHUMAN_BRAVE_API_KEY", "BRAVE_API_KEY"]) {
            if !key.trim().is_empty() {
                self.search.brave.api_key = Some(key);
            }
        }
        if let Some(key) = env.get_any(&["OPENHUMAN_QUERIT_API_KEY", "QUERIT_API_KEY"]) {
            if !key.trim().is_empty() {
                self.search.querit.api_key = Some(key);
            }
        }
        if let Some(key) = env.get_any(&["OPENHUMAN_EXA_API_KEY", "EXA_API_KEY"]) {
            if !key.trim().is_empty() {
                self.search.exa.api_key = Some(key);
            }
        }
        if let Some(key) = env.get_any(&["OPENHUMAN_TAVILY_API_KEY", "TAVILY_API_KEY"]) {
            if !key.trim().is_empty() {
                self.search.tavily.api_key = Some(key);
            }
        }
        if let Some(max) = env.get_any(&["OPENHUMAN_SEARCH_MAX_RESULTS", "SEARCH_MAX_RESULTS"]) {
            if let Ok(n) = max.parse::<usize>() {
                if (1..=20).contains(&n) {
                    self.search.max_results = n;
                }
            }
        }
        if let Some(t) = env.get_any(&["OPENHUMAN_SEARCH_TIMEOUT_SECS", "SEARCH_TIMEOUT_SECS"]) {
            if let Ok(n) = t.parse::<u64>() {
                if n > 0 {
                    self.search.timeout_secs = n;
                }
            }
        }

        if env.contains("OPENHUMAN_WEB_SEARCH_ENABLED") {
            log::warn!(
                "[config] OPENHUMAN_WEB_SEARCH_ENABLED is deprecated and ignored — \
                 web search is always registered; provider/API-key overrides were removed."
            );
        }

        if let Some(max_results) =
            env.get_any(&["OPENHUMAN_WEB_SEARCH_MAX_RESULTS", "WEB_SEARCH_MAX_RESULTS"])
        {
            if let Ok(max_results) = max_results.parse::<usize>() {
                if (1..=10).contains(&max_results) {
                    self.web_search.max_results = max_results;
                }
            }
        }

        if let Some(timeout_secs) = env.get_any(&[
            "OPENHUMAN_WEB_SEARCH_TIMEOUT_SECS",
            "WEB_SEARCH_TIMEOUT_SECS",
        ]) {
            if let Ok(timeout_secs) = timeout_secs.parse::<u64>() {
                if timeout_secs > 0 {
                    self.web_search.timeout_secs = timeout_secs;
                }
            }
        }
    }
}
