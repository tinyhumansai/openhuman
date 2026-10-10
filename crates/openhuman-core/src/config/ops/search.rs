//! Search settings RPC: provider selection and routes, per-role provider
//! order, keys, limits, and the web-access allowlist that sits on the same
//! settings page.
//!
//! Keys are write-only here. Reads report `key_configured` booleans and the
//! resolved state from `crate::search::providers`, never a secret.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::config::{
    Config, SearchPresentation, SearchRoute, MANAGED_SEARCH_PROVIDERS, SEARCH_PROVIDERS,
};
use crate::core::Outcome;
use crate::search::providers::{self, parse_role, role_key, ROLES};

use super::loader::load_config_with_timeout;

/// Partial update of one provider.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SearchProviderPatch {
    pub enabled: Option<bool>,
    /// `managed` or `direct`.
    pub route: Option<String>,
    /// Direct-route key. An empty string clears the stored key.
    pub api_key: Option<String>,
    /// SearXNG instance URL.
    pub base_url: Option<String>,
}

/// Partial update of the search settings. Every field is optional.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SearchSettingsPatch {
    /// Global switch.
    pub enabled: Option<bool>,
    /// Legacy single-engine selection from older clients:
    /// `disabled` | `managed` | `brave` | `querit` | `exa` | `tavily`.
    pub engine: Option<String>,
    /// Per-provider changes, keyed by provider id.
    pub providers: Option<BTreeMap<String, SearchProviderPatch>>,
    /// Ordered provider list per role (`search` | `answer` | `contents`). An
    /// empty list restores the default order.
    pub roles: Option<BTreeMap<String, Vec<String>>>,
    /// `roles` | `all_tools` | `router` | `one_provider`.
    pub presentation: Option<String>,
    /// Provider for `one_provider`; empty clears it.
    pub presentation_provider: Option<String>,
    /// 1..=20.
    pub max_results: Option<usize>,
    /// 1..=120 seconds.
    pub timeout_secs: Option<u64>,
    /// Websites the assistant may open/read (`web_fetch` / `curl`), as a
    /// host allowlist. Entries are exact hosts (`reuters.com`), which also
    /// match their subdomains, or `"*"` for all public sites. Empty list
    /// blocks all web access. Mirrors `[http_request].allowed_domains`.
    pub allowed_domains: Option<Vec<String>>,
    /// "Allow all sites" switch. `Some(true)` sets the allowlist to `["*"]`;
    /// `Some(false)` drops the wildcard while keeping explicit hosts. Applied
    /// after `allowed_domains`.
    pub allow_all: Option<bool>,
}

/// Providers shown in settings. `gemini_deep_research` rides on the Gemini
/// key and is reported under `gemini` rather than as its own row.
const LISTED_PROVIDERS: &[&str] = &[
    "exa", "gemini", "tinyfish", "parallel", "brave", "tavily", "querit", "keenable", "seltz",
    "searxng",
];

fn label(provider: &str) -> String {
    crate::search::render::provider_label(provider)
}

fn docs_url(provider: &str) -> Option<&'static str> {
    match provider {
        "exa" => Some("https://dashboard.exa.ai/api-keys"),
        "gemini" => Some("https://aistudio.google.com/apikey"),
        "parallel" => Some("https://platform.parallel.ai/"),
        "brave" => Some("https://brave.com/search/api/"),
        "tavily" => Some("https://app.tavily.com/"),
        "querit" => Some("https://querit.ai/"),
        "seltz" => Some("https://seltz.ai/"),
        "searxng" => Some("https://docs.searxng.org/"),
        "tinyfish" => Some("https://agent.tinyfish.ai/api-keys"),
        "keenable" => Some("https://keenable.ai/console"),
        _ => None,
    }
}

/// Routes a provider supports, in UI order.
fn supported_routes(provider: &str) -> Vec<&'static str> {
    let managed = MANAGED_SEARCH_PROVIDERS.contains(&provider);
    let mut routes = Vec::new();
    if managed {
        routes.push(SearchRoute::Managed.as_str());
    }
    routes.push(SearchRoute::Direct.as_str());
    routes
}

fn nonempty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn apply_provider_patch(
    config: &mut Config,
    provider: &str,
    patch: SearchProviderPatch,
) -> Result<(), String> {
    if !SEARCH_PROVIDERS.contains(&provider) || provider == "gemini_deep_research" {
        return Err(format!("unknown search provider: {provider}"));
    }
    if let Some(route) = patch.route.as_deref() {
        let route = SearchRoute::parse(route)
            .ok_or_else(|| format!("route must be managed or direct (got {route:?})"))?;
        if !supported_routes(provider).contains(&route.as_str()) {
            return Err(format!(
                "{provider} does not support the {} route",
                route.as_str()
            ));
        }
        let default_enabled = patch.enabled.unwrap_or(true);
        config
            .search
            .providers
            .entry(provider.to_string())
            .or_insert(crate::config::SearchProviderSettings {
                enabled: default_enabled,
                route,
            })
            .route = route;
    }
    if let Some(enabled) = patch.enabled {
        let default_route = if supported_routes(provider)[0] == "managed" {
            SearchRoute::Managed
        } else {
            SearchRoute::Direct
        };
        config
            .search
            .providers
            .entry(provider.to_string())
            .or_insert(crate::config::SearchProviderSettings {
                enabled,
                route: default_route,
            })
            .enabled = enabled;
        match provider {
            "seltz" => config.seltz.enabled = enabled,
            "searxng" => config.searxng.enabled = enabled,
            _ => {}
        }
    }
    if let Some(key) = patch.api_key {
        match provider {
            "seltz" => config.seltz.api_key = nonempty(&key),
            "searxng" => {
                return Err(format!("{provider} does not take an API key"));
            }
            other => {
                config
                    .search
                    .credentials_mut(other)
                    .ok_or_else(|| format!("{other} does not take an API key"))?
                    .api_key = nonempty(&key);
            }
        }
    }
    if let Some(url) = patch.base_url {
        if provider != "searxng" {
            return Err(format!("{provider} does not take a base URL"));
        }
        let url = url.trim();
        if !(url.is_empty() || url.starts_with("http://") || url.starts_with("https://")) {
            return Err("SearXNG base URL must start with http:// or https://".into());
        }
        config.searxng.base_url = url.to_string();
    }
    Ok(())
}

fn apply_roles(config: &mut Config, roles: BTreeMap<String, Vec<String>>) -> Result<(), String> {
    for (key, order) in roles {
        let role = parse_role(&key).ok_or_else(|| {
            format!("unknown search role: {key} (expected search, answer or contents)")
        })?;
        let mut cleaned: Vec<String> = Vec::new();
        for raw in order {
            let provider = raw.trim().to_ascii_lowercase();
            if !tinysearch_bus::provider_roles(&provider).contains(&role) {
                return Err(format!("{provider} cannot serve the {key} role"));
            }
            if !cleaned.contains(&provider) {
                cleaned.push(provider);
            }
        }
        if cleaned.is_empty() {
            config.search.roles.remove(role_key(role));
        } else {
            config
                .search
                .roles
                .insert(role_key(role).to_string(), cleaned);
        }
    }
    Ok(())
}

fn apply_allowlist(config: &mut Config, domains: Option<Vec<String>>, allow_all: Option<bool>) {
    if domains.is_none() && allow_all.is_none() {
        return;
    }
    let before_count = config.http_request.allowed_domains.len();
    let before_allow_all = config.http_request.allowed_domains.iter().any(|d| d == "*");
    if let Some(domains) = domains {
        let mut cleaned: Vec<String> = domains
            .into_iter()
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty())
            .collect();
        cleaned.sort();
        cleaned.dedup();
        config.http_request.allowed_domains = cleaned;
    }
    if let Some(allow_all) = allow_all {
        if allow_all {
            config.http_request.allowed_domains = vec!["*".to_string()];
        } else {
            config.http_request.allowed_domains.retain(|d| d != "*");
        }
    }
    tracing::info!(
        before_count,
        after_count = config.http_request.allowed_domains.len(),
        before_allow_all,
        after_allow_all = config.http_request.allowed_domains.iter().any(|d| d == "*"),
        "[config] http_request.allowed_domains updated"
    );
}

/// Apply a patch in memory, validating every field before anything is saved.
pub fn apply_search_patch(config: &mut Config, update: SearchSettingsPatch) -> Result<(), String> {
    if let Some(engine) = update.engine.as_deref() {
        config.search.apply_legacy_engine(engine)?;
    }
    if let Some(enabled) = update.enabled {
        config.search.enabled = Some(enabled);
    }
    if let Some(patches) = update.providers {
        for (provider, patch) in patches {
            apply_provider_patch(config, provider.trim(), patch)?;
        }
    }
    if let Some(roles) = update.roles {
        apply_roles(config, roles)?;
    }
    if let Some(mode) = update.presentation.as_deref() {
        config.search.presentation = SearchPresentation::parse(mode).ok_or_else(|| {
            "presentation must be roles, all_tools, router or one_provider".to_string()
        })?;
    }
    if let Some(provider) = update.presentation_provider {
        let provider = provider.trim().to_ascii_lowercase();
        if !provider.is_empty() && !SEARCH_PROVIDERS.contains(&provider.as_str()) {
            return Err(format!("unknown search presentation provider: {provider}"));
        }
        config.search.presentation_provider = (!provider.is_empty()).then_some(provider);
    }
    if let Some(n) = update.max_results {
        if !(1..=20).contains(&n) {
            return Err(format!("max_results must be between 1 and 20 (got {n})"));
        }
        config.search.max_results = n;
    }
    if let Some(secs) = update.timeout_secs {
        if !(1..=120).contains(&secs) {
            return Err(format!(
                "timeout_secs must be between 1 and 120 (got {secs})"
            ));
        }
        config.search.timeout_secs = secs;
    }
    apply_allowlist(config, update.allowed_domains, update.allow_all);
    Ok(())
}

/// Validate, save, refresh the module, and return the new settings.
pub async fn apply_search_settings(
    config: &mut Config,
    update: SearchSettingsPatch,
) -> Result<Outcome<Value>, String> {
    apply_search_patch(config, update)?;
    config.save().await.map_err(|e| e.to_string())?;
    #[cfg(feature = "modules")]
    crate::modules::search::refresh_loaded(config).await?;
    tracing::debug!(
        enabled = config.search.is_enabled(),
        providers = ?config.search.enabled_provider_names(),
        "[config][search] settings saved"
    );
    Ok(Outcome::new(
        search_settings_json(config),
        vec![format!(
            "search settings saved to {}",
            config.config_path.display()
        )],
    ))
}

pub async fn load_and_apply_search_settings(
    update: SearchSettingsPatch,
) -> Result<Outcome<Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_search_settings(&mut config, update).await
}

/// The settings view: every listed provider's resolved state, the role
/// orders, and what actually serves each role right now.
pub fn search_settings_json(config: &Config) -> Value {
    search_settings_json_with(config, providers::backend_credential_available(config))
}

pub(crate) fn search_settings_json_with(config: &Config, managed_available: bool) -> Value {
    let resolved = providers::resolve_with(config, managed_available);
    let search_enabled = config.search.is_enabled();
    let gemini_key = config.search.gemini.has_key();
    let providers_json: Vec<Value> = LISTED_PROVIDERS
        .iter()
        .filter_map(|id| resolved.iter().find(|p| p.id == *id))
        .map(|p| {
            let mut entry = json!({
                "id": p.id,
                "label": label(p.id),
                "enabled": p.enabled,
                "route": p.route.as_str(),
                "routes": supported_routes(p.id),
                "managed_available": p.managed_available,
                "key_configured": p.key_configured,
                "takes_key": p.id != "searxng",
                "key_optional": p.key_optional,
                "usable": p.usable,
                "status": p.status(search_enabled),
                "roles": p.roles,
                "docs_url": docs_url(p.id),
            });
            if p.id == "gemini" {
                entry["deep_research_available"] = json!(search_enabled && p.enabled && gemini_key);
            }
            if p.id == "searxng" {
                entry["base_url"] = json!(config.searxng.base_url);
            }
            entry
        })
        .collect();
    let roles: serde_json::Map<String, Value> = ROLES
        .into_iter()
        .map(|role| {
            (
                role_key(role).to_string(),
                json!(providers::role_order(config, role)),
            )
        })
        .collect();
    let effective_roles: serde_json::Map<String, Value> = ROLES
        .into_iter()
        .map(|role| {
            (
                role_key(role).to_string(),
                json!(providers::effective_role_providers(&resolved, config, role)),
            )
        })
        .collect();
    json!({
        "enabled": search_enabled,
        "presentation": config.search.presentation.as_str(),
        "presentation_provider": config.search.presentation_provider,
        "max_results": config.search.max_results,
        "timeout_secs": config.search.timeout_secs,
        "managed_available": managed_available,
        "providers": providers_json,
        "roles": roles,
        "effective_roles": effective_roles,
        "allowed_domains": config.http_request.allowed_domains,
        "allow_all": config.http_request.allowed_domains.iter().any(|d| d == "*"),
    })
}

/// Read the current search settings. Keys are reported only as booleans.
pub async fn get_search_settings() -> Result<Outcome<Value>, String> {
    let config = load_config_with_timeout().await?;
    Ok(Outcome::new(
        search_settings_json(&config),
        vec!["search settings read".to_string()],
    ))
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
