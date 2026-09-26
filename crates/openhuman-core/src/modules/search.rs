//! Host adapter for the TinySearch module. Secrets enter only through private
//! TinyBus initialization and reinitialization payloads.

use std::{
    collections::BTreeMap,
    future::Future,
    hash::{Hash, Hasher},
    sync::OnceLock,
};
use tinysearch_bus::{
    names, BackendAuthMode, BackendConfig, ExecuteToolRequest, ExecuteToolResponse,
    ListToolsResponse, PresentationConfig, PresentationMode, ProviderConfig, ProviderRoute,
    SearchConfig as ModuleSearchConfig,
};

use crate::config::Config;

pub const MODULE_ID: &str = "tinysearch";

fn provider(
    enabled: bool,
    credential: Option<&str>,
    route: ProviderRoute,
    max_results: Option<u64>,
    timeout_secs: Option<u64>,
) -> ProviderConfig {
    ProviderConfig {
        enabled,
        credential: credential.map(str::to_owned),
        route,
        max_results,
        timeout_secs,
        ..Default::default()
    }
}

/// The complete private configuration for TinySearch. No credentials are
/// returned over settings RPC or sent on ordinary bus method calls.
pub fn module_config(config: &Config) -> ModuleSearchConfig {
    let credential =
        crate::security::credentials::session_support::resolve_backend_credential(config).ok();
    let selected = config.search.providers(
        credential.is_some(),
        config.integrations.tinyfish.is_active(),
        config.seltz.enabled
            && config
                .seltz
                .api_key
                .as_deref()
                .is_some_and(|key| !key.trim().is_empty()),
        config.searxng.enabled,
    );
    let backend = BackendConfig {
        base_url: Some(crate::api::effective_backend_api_url(&config.api_url)),
        auth_mode: if credential.as_ref().is_some_and(|c| c.is_api_key()) {
            BackendAuthMode::ApiKey
        } else {
            BackendAuthMode::Session
        },
        credential: credential.map(|c| c.into_secret()),
        sdk_name: Some(crate::api::product_identity().as_str().to_owned()),
    };
    let mut providers = BTreeMap::new();
    let limits = (
        Some(config.search.max_results as u64),
        Some(config.search.timeout_secs),
    );
    for (name, key) in [
        ("brave", config.search.brave.key()),
        ("querit", config.search.querit.key()),
        ("exa", config.search.exa.key()),
        ("tavily", config.search.tavily.key()),
        ("gemini", config.search.gemini.key()),
        ("parallel", config.search.parallel.key()),
    ] {
        let route = if (name == "parallel" && config.search.parallel_route == "backend")
            || (name == "gemini" && config.search.gemini_route == "backend")
        {
            ProviderRoute::Backend
        } else {
            ProviderRoute::Direct
        };
        let enabled = selected.contains(name)
            && (key.is_some() || (route == ProviderRoute::Backend && backend.credential.is_some()));
        providers.insert(
            name.into(),
            provider(enabled, key, route, limits.0, limits.1),
        );
    }
    // TinySearch names managed search `parallel` with its backend route.
    // A usable direct Parallel route takes precedence until the bus can
    // represent both routes of that provider at once. If no direct key is
    // present, preserve the managed backend route.
    if selected.contains("managed")
        && (!selected.contains("parallel")
            || (config.search.parallel_route != "backend" && !config.search.parallel.has_key()))
    {
        providers.insert(
            "parallel".into(),
            provider(
                backend.credential.is_some(),
                None,
                ProviderRoute::Backend,
                limits.0,
                limits.1,
            ),
        );
    }
    providers.insert(
        "tinyfish".into(),
        provider(
            selected.contains("tinyfish")
                && (config.search.enabled_providers.is_some()
                    || config.integrations.tinyfish.is_active()),
            None,
            ProviderRoute::Backend,
            limits.0,
            limits.1,
        ),
    );
    // Deep Research is direct-only even when grounded Gemini search takes
    // the managed backend route.
    providers.insert(
        "gemini_deep_research".into(),
        provider(
            selected.contains("gemini") && config.search.gemini.has_key(),
            config.search.gemini.key(),
            ProviderRoute::Direct,
            limits.0,
            limits.1,
        ),
    );
    providers.insert(
        "seltz".into(),
        ProviderConfig {
            enabled: selected.contains("seltz")
                && config
                    .seltz
                    .api_key
                    .as_deref()
                    .is_some_and(|key| !key.trim().is_empty()),
            credential: config.seltz.api_key.clone(),
            base_url: config.seltz.api_url.clone(),
            max_results: Some(config.seltz.max_results as u64),
            timeout_secs: Some(config.seltz.timeout_secs),
            ..Default::default()
        },
    );
    providers.insert(
        "searxng".into(),
        ProviderConfig {
            enabled: selected.contains("searxng"),
            base_url: Some(config.searxng.base_url.clone()),
            max_results: Some(config.searxng.max_results as u64),
            timeout_secs: Some(config.searxng.timeout_secs),
            default_language: Some(config.searxng.default_language.clone()),
            ..Default::default()
        },
    );
    ModuleSearchConfig {
        enabled: config.search.is_enabled(),
        backend,
        providers,
        presentation: PresentationConfig {
            mode: match config.search.presentation.as_str() {
                "router" => PresentationMode::Router,
                "one_provider" => PresentationMode::OneProvider,
                _ => PresentationMode::AllTools,
            },
            provider: config.search.presentation_provider.as_deref().map(|name| {
                if name == "managed" {
                    "parallel".to_owned()
                } else {
                    name.to_owned()
                }
            }),
        },
    }
}

/// Deterministic declarations for synchronous tool registration. Invocation
/// still refreshes the module and lets it validate the current tool set.
pub fn configured_tool_specs(config: &Config) -> Vec<tinysearch_bus::ToolSpec> {
    let module = module_config(config);
    let available =
        tinysearch_bus::configured_provider_tools(&module, &tinysearch_bus::provider_tool_specs());
    tinysearch_bus::select_tools(&available, &module.presentation).tools
}

fn last_config() -> &'static tokio::sync::Mutex<Option<u64>> {
    static LAST: OnceLock<tokio::sync::Mutex<Option<u64>>> = OnceLock::new();
    LAST.get_or_init(|| tokio::sync::Mutex::new(None))
}

fn fingerprint(value: &serde_json::Value) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.to_string().hash(&mut hasher);
    hasher.finish()
}

async fn proxy(config: &Config) -> Result<tinybus::Proxy, String> {
    super::ops::ensure_loaded_within(config, MODULE_ID, Some(std::time::Duration::from_secs(8)))
        .await
        .map_err(super::ops::LoadError::into_message)?;
    let runtime = super::host::runtime()
        .await
        .map_err(|error| format!("search module bus unavailable: {error}"))?;
    let configuration =
        serde_json::to_value(module_config(config)).map_err(|error| error.to_string())?;
    let current = fingerprint(&configuration);
    // Caller holds the module call lock through the subsequent bus call.
    let mut previous = last_config().lock().await;
    if *previous != Some(current) {
        runtime
            .connection()
            .reinitialize_module(MODULE_ID, configuration)
            .await
            .map_err(|error| format!("search module configuration refresh failed: {error}"))?;
        *previous = Some(current);
    }
    runtime
        .proxy(names::INTERFACE, names::OBJECT_PATH)
        .map_err(|error| format!("search module proxy unavailable: {error}"))
}

async fn with_module_lock<T, F, Fut>(operation: F) -> Result<T, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    static CALL_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    let guard = CALL_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let result = operation().await;
    drop(guard);
    result
}

async fn current_config(config: &Config) -> Result<Config, String> {
    crate::config::ops::reload_config_from_paths(&config.config_path, &config.workspace_dir).await
}

/// Refresh a loaded module after a settings or login change. An unloaded
/// module gets the current configuration during its first initialization.
pub async fn refresh_loaded(config: &Config) -> Result<(), String> {
    if matches!(
        super::ops::state_of(MODULE_ID),
        super::types::ModuleState::Ready
    ) {
        let current = current_config(config).await?;
        with_module_lock(|| async {
            let _ = proxy(&current).await?;
            Ok(())
        })
        .await?;
    }
    Ok(())
}

pub async fn list_tools(config: &Config) -> Result<ListToolsResponse, String> {
    let current = current_config(config).await?;
    with_module_lock(|| async {
        proxy(&current)
            .await?
            .call(names::methods::LIST_TOOLS, ())
            .await
            .map_err(|error| format!("search ListTools failed: {error}"))
    })
    .await
}

pub async fn execute_tool(
    config: &Config,
    request: ExecuteToolRequest,
) -> Result<ExecuteToolResponse, String> {
    let current = current_config(config).await?;
    with_module_lock(|| async {
        proxy(&current)
            .await?
            .call_confidential(names::methods::EXECUTE_TOOL, (request,))
            .await
            .map_err(|error| format!("search ExecuteTool failed: {error}"))
    })
    .await
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
