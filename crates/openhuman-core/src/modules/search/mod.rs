//! Host adapter for the TinySearch module.
//!
//! `module_config` turns the host's resolved search policy
//! (`crate::search::providers`) into the module's private configuration.
//! Secrets enter only through that configuration, delivered at TinyBus
//! initialization and reinitialization; they never ride an ordinary method
//! call or a settings RPC response. A key an embedder pinned on one agent's
//! config ([`crate::config::SearchHostCredential`]) enters the same way, for
//! that agent's calls. `proxy` holds the call path.

use std::collections::BTreeMap;

use tinysearch_bus::{
    BackendAuthMode, BackendConfig, PresentationConfig, ProviderConfig, ProviderRoute,
    SearchConfig as ModuleSearchConfig,
};

use crate::config::Config;
use crate::search::providers::{self, ResolvedProvider};
use tinysearch_bus::ROLES;

mod proxy;

pub use proxy::{execute_tool, list_tools, refresh_loaded};

/// [`module_config`] as the JSON the module is loaded with, recorded as the
/// configuration the module holds so the first call knows whether to
/// reconfigure it.
pub(crate) fn load_configuration(config: &Config) -> serde_json::Value {
    let configuration =
        serde_json::to_value(module_config(config)).expect("TinySearch config serializes");
    proxy::record_load_configuration(&configuration);
    configuration
}

pub const MODULE_ID: &str = "tinysearch";

/// Private direct-route settings for one provider: key, base URL, limits.
fn direct_settings(config: &Config, provider: &str) -> ProviderConfig {
    let search = &config.search;
    let mut settings = ProviderConfig {
        enabled: true,
        route: ProviderRoute::Direct,
        max_results: Some(search.max_results as u64),
        timeout_secs: Some(search.timeout_secs),
        ..Default::default()
    };
    match provider {
        "seltz" => {
            settings.credential = config
                .seltz
                .api_key
                .clone()
                .filter(|k| !k.trim().is_empty());
            settings.base_url = config.seltz.api_url.clone();
            settings.max_results = Some(config.seltz.max_results as u64);
            settings.timeout_secs = Some(config.seltz.timeout_secs);
        }
        "searxng" => {
            settings.base_url = Some(config.searxng.base_url.clone());
            settings.max_results = Some(config.searxng.max_results as u64);
            settings.timeout_secs = Some(config.searxng.timeout_secs);
            settings.default_language = Some(config.searxng.default_language.clone());
        }
        other => {
            // A key pinned on this agent's config wins over the stored one.
            settings.credential = search.direct_key(other).map(str::to_owned);
            settings.base_url = search
                .host_credential(other)
                .and_then(|pinned| pinned.base_url())
                .or_else(|| {
                    search
                        .credentials(other)
                        .and_then(|c| c.base_url.as_deref())
                })
                .map(str::to_owned);
        }
    }
    settings
}

fn provider_config(config: &Config, provider: &ResolvedProvider) -> ProviderConfig {
    let mut settings = match provider.route {
        ProviderRoute::Backend => ProviderConfig {
            enabled: true,
            route: ProviderRoute::Backend,
            max_results: Some(config.search.max_results as u64),
            timeout_secs: Some(config.search.timeout_secs),
            ..Default::default()
        },
        ProviderRoute::Direct => direct_settings(config, provider.id),
    };
    settings.enabled = provider.usable;
    settings
}

/// The complete private configuration for TinySearch.
pub fn module_config(config: &Config) -> ModuleSearchConfig {
    let private = config
        .profile_providers
        .as_ref()
        .map(|binding| binding.effective_search(config));
    let config = private.as_ref().unwrap_or(config);
    let direct_profile = config.profile_providers.is_some();
    let credential = if direct_profile {
        None
    } else {
        crate::security::credentials::session_support::resolve_backend_credential(config).ok()
    };
    let resolved = providers::resolve_with(config, credential.is_some());
    let backend = if direct_profile {
        BackendConfig::default()
    } else {
        BackendConfig {
            base_url: config.resolved_endpoints.control_plane_url.clone(),
            auth_mode: if credential.as_ref().is_some_and(|c| c.is_api_key()) {
                BackendAuthMode::ApiKey
            } else {
                BackendAuthMode::Session
            },
            credential: credential.map(|c| c.into_secret()),
            sdk_name: config.resolved_endpoints.product_identity.clone(),
        }
    };

    let mut module_providers: BTreeMap<String, ProviderConfig> = resolved
        .iter()
        .filter(|provider| provider.enabled)
        .map(|provider| (provider.id.to_string(), provider_config(config, provider)))
        .collect();
    // Deep research runs on the direct Gemini API and needs the user's key,
    // even when grounded Gemini answers go through the managed route.
    if !direct_profile
        && module_providers.contains_key("gemini")
        && !module_providers.contains_key("gemini_deep_research")
    {
        let mut deep = direct_settings(config, "gemini_deep_research");
        deep.enabled = crate::search::is_active(config) && deep.credential.is_some();
        module_providers.insert("gemini_deep_research".into(), deep);
    }

    let roles = ROLES
        .into_iter()
        .map(|role| (role, providers::role_order(config, role)))
        .collect();
    let presentation = PresentationConfig {
        mode: config.search.presentation,
        provider: config.search.presentation_provider.clone(),
        roles,
    };

    tracing::debug!(
        enabled = crate::search::is_active(config),
        providers = ?module_providers
            .iter()
            .map(|(name, p)| format!("{name}:{:?}:{}", p.route, p.enabled))
            .collect::<Vec<_>>(),
        presentation = config.search.presentation.as_str(),
        "[modules][search] built module configuration"
    );

    ModuleSearchConfig {
        enabled: crate::search::is_active(config),
        backend,
        providers: module_providers,
        presentation,
    }
}

/// Deterministic declarations for synchronous tool registration, computed
/// from the same configuration the module will receive.
pub fn configured_tool_specs(config: &Config) -> Vec<tinysearch_bus::ToolSpec> {
    let module = module_config(config);
    if !module.enabled {
        return Vec::new();
    }
    let available =
        tinysearch_bus::configured_provider_tools(&module, &tinysearch_bus::provider_tool_specs());
    let mut specs = tinysearch_bus::select_tools(&available, &module.presentation).tools;
    if let Some(binding) = &config.profile_providers {
        binding.present(&mut specs);
    }
    specs
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod config_tests;
