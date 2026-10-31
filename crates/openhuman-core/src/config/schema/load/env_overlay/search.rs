//! Env overrides for web search: provider set, routes, roles, keys, and the
//! Seltz/SearXNG options.

use crate::config::schema::load::env::parse_env_bool;
use crate::config::schema::load::env::EnvLookup;
use crate::config::schema::{
    Config, PresentationMode, ProviderRoute, SearchProviderSettings, MANAGED_SEARCH_PROVIDERS,
    SEARCH_PROVIDERS, SEARCH_ROLES,
};

impl Config {
    pub(super) fn apply_search_env<E: EnvLookup + ?Sized>(&mut self, env: &E) {
        if let Some(key) = env.get_any(&["OPENHUMAN_SELTZ_API_KEY", "SELTZ_API_KEY"]) {
            if !key.is_empty() {
                self.seltz.api_key = Some(key);
                self.seltz.enabled = true;
                self.record_env_override(
                    env,
                    "seltz.api_key",
                    &["OPENHUMAN_SELTZ_API_KEY", "SELTZ_API_KEY"],
                );
                self.record_env_override(
                    env,
                    "seltz.enabled",
                    &["OPENHUMAN_SELTZ_API_KEY", "SELTZ_API_KEY"],
                );
                self.search
                    .providers
                    .insert("seltz".into(), SearchProviderSettings::direct());
                self.record_env_override(
                    env,
                    "search.providers.seltz",
                    &["OPENHUMAN_SELTZ_API_KEY", "SELTZ_API_KEY"],
                );
            }
        }
        if let Some(url) = env.get_any(&["OPENHUMAN_SELTZ_API_URL", "SELTZ_API_URL"]) {
            if !url.is_empty() {
                self.seltz.api_url = Some(url);
                self.record_env_override(
                    env,
                    "seltz.api_url",
                    &["OPENHUMAN_SELTZ_API_URL", "SELTZ_API_URL"],
                );
            }
        }
        if let Some(max) = env.get_any(&["OPENHUMAN_SELTZ_MAX_RESULTS", "SELTZ_MAX_RESULTS"]) {
            if let Ok(n) = max.parse::<usize>() {
                if (1..=20).contains(&n) {
                    self.seltz.max_results = n;
                    self.record_env_override(
                        env,
                        "seltz.max_results",
                        &["OPENHUMAN_SELTZ_MAX_RESULTS", "SELTZ_MAX_RESULTS"],
                    );
                }
            }
        }

        if let Some(flag) = env.get_any(&["OPENHUMAN_SEARXNG_ENABLED", "SEARXNG_ENABLED"]) {
            if let Some(enabled) = parse_env_bool("OPENHUMAN_SEARXNG_ENABLED", &flag) {
                self.searxng.enabled = enabled;
                self.record_env_override(
                    env,
                    "searxng.enabled",
                    &["OPENHUMAN_SEARXNG_ENABLED", "SEARXNG_ENABLED"],
                );
                if enabled {
                    self.search
                        .providers
                        .insert("searxng".into(), SearchProviderSettings::direct());
                    self.record_env_override(
                        env,
                        "search.providers.searxng",
                        &["OPENHUMAN_SEARXNG_ENABLED", "SEARXNG_ENABLED"],
                    );
                } else {
                    self.search.providers.remove("searxng");
                    self.record_env_override(
                        env,
                        "search.providers.searxng",
                        &["OPENHUMAN_SEARXNG_ENABLED", "SEARXNG_ENABLED"],
                    );
                }
            }
        }
        if let Some(url) = env.get_any(&["OPENHUMAN_SEARXNG_BASE_URL", "SEARXNG_BASE_URL"]) {
            let url = url.trim();
            if !url.is_empty() {
                self.searxng.base_url = url.to_string();
                self.record_env_override(
                    env,
                    "searxng.base_url",
                    &["OPENHUMAN_SEARXNG_BASE_URL", "SEARXNG_BASE_URL"],
                );
            }
        }
        if let Some(max) = env.get_any(&["OPENHUMAN_SEARXNG_MAX_RESULTS", "SEARXNG_MAX_RESULTS"]) {
            if let Ok(n) = max.parse::<usize>() {
                if (1..=50).contains(&n) {
                    self.searxng.max_results = n;
                    self.record_env_override(
                        env,
                        "searxng.max_results",
                        &["OPENHUMAN_SEARXNG_MAX_RESULTS", "SEARXNG_MAX_RESULTS"],
                    );
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
                self.record_env_override(
                    env,
                    "searxng.default_language",
                    &[
                        "OPENHUMAN_SEARXNG_DEFAULT_LANGUAGE",
                        "SEARXNG_DEFAULT_LANGUAGE",
                    ],
                );
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
                    self.record_env_override(
                        env,
                        "searxng.timeout_secs",
                        &[
                            "OPENHUMAN_SEARXNG_TIMEOUT_SECS",
                            "OPENHUMAN_SEARXNG_TIMEOUT_SECONDS",
                            "SEARXNG_TIMEOUT_SECS",
                            "SEARXNG_TIMEOUT_SECONDS",
                        ],
                    );
                }
            }
        }

        if let Some(flag) = env.get_any(&["OPENHUMAN_SEARCH_ENABLED"]) {
            if let Some(enabled) = parse_env_bool("OPENHUMAN_SEARCH_ENABLED", &flag) {
                self.search.enabled = Some(enabled);
                self.record_env_override(env, "search.enabled", &["OPENHUMAN_SEARCH_ENABLED"]);
            }
        }
        if let Some(engine) = env.get_any(&["OPENHUMAN_SEARCH_ENGINE", "SEARCH_ENGINE"]) {
            if !engine.trim().is_empty() {
                if let Err(error) = self.search.apply_legacy_engine(&engine) {
                    log::warn!("[config][search] ignoring SEARCH_ENGINE: {error}");
                } else {
                    self.record_env_override(
                        env,
                        "search.providers",
                        &["OPENHUMAN_SEARCH_ENGINE", "SEARCH_ENGINE"],
                    );
                }
            }
        }
        // `exa:managed,gemini,brave` — replaces the provider set. A provider
        // without a route is managed when it can be, direct otherwise.
        if let Some(names) = env.get_any(&["OPENHUMAN_SEARCH_PROVIDERS"]) {
            let mut providers = std::collections::BTreeMap::new();
            for entry in names.split(',').map(str::trim).filter(|e| !e.is_empty()) {
                let (name, route) = match entry.split_once(':') {
                    Some((name, route)) => (
                        name.trim().to_ascii_lowercase(),
                        ProviderRoute::parse(route),
                    ),
                    None => (entry.to_ascii_lowercase(), None),
                };
                if name == "managed" {
                    providers.insert("exa".to_string(), SearchProviderSettings::managed());
                    providers.insert("gemini".to_string(), SearchProviderSettings::managed());
                    continue;
                }
                if !SEARCH_PROVIDERS.contains(&name.as_str()) {
                    log::warn!(
                        "[config][search] OPENHUMAN_SEARCH_PROVIDERS: unknown provider '{name}'"
                    );
                    continue;
                }
                let default_route = if MANAGED_SEARCH_PROVIDERS.contains(&name.as_str()) {
                    ProviderRoute::Backend
                } else {
                    ProviderRoute::Direct
                };
                providers.insert(
                    name,
                    SearchProviderSettings {
                        enabled: true,
                        route: route.unwrap_or(default_route),
                    },
                );
            }
            self.search.providers = providers;
            self.record_env_override(env, "search.providers", &["OPENHUMAN_SEARCH_PROVIDERS"]);
        }
        // The dedicated provider toggles and credentials take precedence over
        // the provider-set shorthand, which replaces the map above.
        if let Some(key) = env.get_any(&["OPENHUMAN_SELTZ_API_KEY", "SELTZ_API_KEY"]) {
            if !key.is_empty() {
                self.search
                    .providers
                    .insert("seltz".into(), SearchProviderSettings::direct());
                self.record_env_override(
                    env,
                    "search.providers.seltz",
                    &["OPENHUMAN_SELTZ_API_KEY", "SELTZ_API_KEY"],
                );
            }
        }
        if let Some(flag) = env.get_any(&["OPENHUMAN_SEARXNG_ENABLED", "SEARXNG_ENABLED"]) {
            if let Some(enabled) = parse_env_bool("OPENHUMAN_SEARXNG_ENABLED", &flag) {
                if enabled {
                    self.search
                        .providers
                        .insert("searxng".into(), SearchProviderSettings::direct());
                    self.record_env_override(
                        env,
                        "search.providers.searxng",
                        &["OPENHUMAN_SEARXNG_ENABLED", "SEARXNG_ENABLED"],
                    );
                } else {
                    self.search.providers.remove("searxng");
                    self.record_env_override(
                        env,
                        "search.providers.searxng",
                        &["OPENHUMAN_SEARXNG_ENABLED", "SEARXNG_ENABLED"],
                    );
                }
            }
        }
        // `exa=direct,gemini=managed` — changes routes of listed providers.
        let mut routes: Vec<(String, String, &str)> = Vec::new();
        if let Some(value) = env.get_any(&["OPENHUMAN_SEARCH_ROUTES"]) {
            routes.extend(value.split(',').filter_map(|pair| {
                pair.split_once('=').map(|(p, r)| {
                    (
                        p.trim().to_ascii_lowercase(),
                        r.trim().to_string(),
                        "OPENHUMAN_SEARCH_ROUTES",
                    )
                })
            }));
        }
        if let Some(value) = env.get_any(&["OPENHUMAN_GEMINI_ROUTE"]) {
            log::warn!(
                "[config][search] OPENHUMAN_GEMINI_ROUTE is deprecated; use OPENHUMAN_SEARCH_ROUTES=gemini=<route>"
            );
            routes.push(("gemini".into(), value, "OPENHUMAN_GEMINI_ROUTE"));
        }
        for (provider, route, env_var) in routes {
            match ProviderRoute::parse(&route) {
                Some(route)
                    if SEARCH_PROVIDERS.contains(&provider.as_str())
                        && self.search.providers.contains_key(&provider) =>
                {
                    self.search
                        .providers
                        .get_mut(&provider)
                        .expect("checked above")
                        .route = route;
                    self.record_env_override_var(
                        env,
                        &format!("search.providers.{provider}.route"),
                        env_var,
                    );
                }
                _ => log::warn!("[config][search] ignoring route '{provider}={route}'"),
            }
        }
        // `search=brave|exa;answer=gemini` — ordered provider list per role.
        if let Some(value) = env.get_any(&["OPENHUMAN_SEARCH_ROLES"]) {
            for spec in value.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                let Some((role, list)) = spec.split_once('=') else {
                    log::warn!("[config][search] OPENHUMAN_SEARCH_ROLES: malformed '{spec}'");
                    continue;
                };
                let role = role.trim().to_ascii_lowercase();
                if !SEARCH_ROLES.contains(&role.as_str()) {
                    log::warn!("[config][search] OPENHUMAN_SEARCH_ROLES: unknown role '{role}'");
                    continue;
                }
                let order: Vec<String> = list
                    .split('|')
                    .map(|p| p.trim().to_ascii_lowercase())
                    .filter(|p| SEARCH_PROVIDERS.contains(&p.as_str()))
                    .collect();
                self.search.roles.insert(role, order);
                self.record_env_override(env, "search.roles", &["OPENHUMAN_SEARCH_ROLES"]);
            }
        }
        if let Some(mode) = env.get_any(&["OPENHUMAN_SEARCH_PRESENTATION"]) {
            match PresentationMode::parse(&mode) {
                Some(mode) => {
                    self.search.presentation = mode;
                    self.record_env_override(
                        env,
                        "search.presentation",
                        &["OPENHUMAN_SEARCH_PRESENTATION"],
                    );
                }
                None => {
                    log::warn!("[config][search] ignoring OPENHUMAN_SEARCH_PRESENTATION='{mode}'")
                }
            }
        }
        if env.contains("OPENHUMAN_PARALLEL_ROUTE") {
            log::warn!(
                "[config][search] managed Parallel is no longer offered; OPENHUMAN_PARALLEL_ROUTE is ignored (Parallel uses your own key)"
            );
        }
        if let Some(key) = env.get_any(&["OPENHUMAN_PARALLEL_API_KEY", "PARALLEL_API_KEY"]) {
            if !key.trim().is_empty() {
                self.search.parallel.api_key = Some(key);
                self.record_env_override(
                    env,
                    "search.parallel.api_key",
                    &["OPENHUMAN_PARALLEL_API_KEY", "PARALLEL_API_KEY"],
                );
            }
        }
        if let Some(key) = env.get_any(&["OPENHUMAN_GEMINI_API_KEY", "GEMINI_API_KEY"]) {
            if !key.trim().is_empty() {
                self.search.gemini.api_key = Some(key);
                self.record_env_override(
                    env,
                    "search.gemini.api_key",
                    &["OPENHUMAN_GEMINI_API_KEY", "GEMINI_API_KEY"],
                );
            }
        }
        if let Some(key) = env.get_any(&["OPENHUMAN_BRAVE_API_KEY", "BRAVE_API_KEY"]) {
            if !key.trim().is_empty() {
                self.search.brave.api_key = Some(key);
                self.record_env_override(
                    env,
                    "search.brave.api_key",
                    &["OPENHUMAN_BRAVE_API_KEY", "BRAVE_API_KEY"],
                );
            }
        }
        if let Some(key) = env.get_any(&["OPENHUMAN_QUERIT_API_KEY", "QUERIT_API_KEY"]) {
            if !key.trim().is_empty() {
                self.search.querit.api_key = Some(key);
                self.record_env_override(
                    env,
                    "search.querit.api_key",
                    &["OPENHUMAN_QUERIT_API_KEY", "QUERIT_API_KEY"],
                );
            }
        }
        if let Some(key) = env.get_any(&["OPENHUMAN_EXA_API_KEY", "EXA_API_KEY"]) {
            if !key.trim().is_empty() {
                self.search.exa.api_key = Some(key);
                self.record_env_override(
                    env,
                    "search.exa.api_key",
                    &["OPENHUMAN_EXA_API_KEY", "EXA_API_KEY"],
                );
            }
        }
        if let Some(key) = env.get_any(&["OPENHUMAN_TAVILY_API_KEY", "TAVILY_API_KEY"]) {
            if !key.trim().is_empty() {
                self.search.tavily.api_key = Some(key);
                self.record_env_override(
                    env,
                    "search.tavily.api_key",
                    &["OPENHUMAN_TAVILY_API_KEY", "TAVILY_API_KEY"],
                );
            }
        }
        if let Some(key) = env.get_any(&["OPENHUMAN_TINYFISH_API_KEY", "TINYFISH_API_KEY"]) {
            if !key.trim().is_empty() {
                self.search.tinyfish.api_key = Some(key);
                self.record_env_override(
                    env,
                    "search.tinyfish.api_key",
                    &["OPENHUMAN_TINYFISH_API_KEY", "TINYFISH_API_KEY"],
                );
            }
        }
        if let Some(key) = env.get_any(&["OPENHUMAN_KEENABLE_API_KEY", "KEENABLE_API_KEY"]) {
            if !key.trim().is_empty() {
                self.search.keenable.api_key = Some(key);
            }
        }
        if let Some(max) = env.get_any(&["OPENHUMAN_SEARCH_MAX_RESULTS", "SEARCH_MAX_RESULTS"]) {
            if let Ok(n) = max.parse::<usize>() {
                if (1..=20).contains(&n) {
                    self.search.max_results = n;
                    self.record_env_override(
                        env,
                        "search.max_results",
                        &["OPENHUMAN_SEARCH_MAX_RESULTS", "SEARCH_MAX_RESULTS"],
                    );
                }
            }
        }
        if let Some(t) = env.get_any(&["OPENHUMAN_SEARCH_TIMEOUT_SECS", "SEARCH_TIMEOUT_SECS"]) {
            if let Ok(n) = t.parse::<u64>() {
                if n > 0 {
                    self.search.timeout_secs = n;
                    self.record_env_override(
                        env,
                        "search.timeout_secs",
                        &["OPENHUMAN_SEARCH_TIMEOUT_SECS", "SEARCH_TIMEOUT_SECS"],
                    );
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
                    self.record_env_override(
                        env,
                        "web_search.max_results",
                        &["OPENHUMAN_WEB_SEARCH_MAX_RESULTS", "WEB_SEARCH_MAX_RESULTS"],
                    );
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
                    self.record_env_override(
                        env,
                        "web_search.timeout_secs",
                        &[
                            "OPENHUMAN_WEB_SEARCH_TIMEOUT_SECS",
                            "WEB_SEARCH_TIMEOUT_SECS",
                        ],
                    );
                }
            }
        }
    }
}
