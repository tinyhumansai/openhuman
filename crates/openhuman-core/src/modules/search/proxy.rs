//! Call path to the loaded TinySearch module: lazy load, private
//! reconfiguration when the resolved configuration changes, and serialized
//! `ListTools` / `ExecuteTool` calls.
//!
//! The module is one per process, but a call's configuration is the caller's:
//! the reloaded `config.toml` for an ordinary agent, or the agent's own
//! in-memory config when an embedder pinned search keys on it
//! ([`crate::config::SearchConfig::pin_host_credential`]). Before each call
//! the module is reconfigured if it holds anything else, and the call lock is
//! held through the call, so agents with different keys take turns rather
//! than ever sharing one configuration.

use std::{
    future::Future,
    hash::{Hash, Hasher},
    sync::OnceLock,
};

use tinysearch_bus::{names, ExecuteToolRequest, ExecuteToolResponse, ListToolsResponse};

use super::{module_config, MODULE_ID};
use crate::config::Config;
use crate::core::runtime::DispatchCtx;

/// Bus deadline for one `ExecuteTool` call.
///
/// The bus default ([`tinybus::connection::DEFAULT_TIMEOUT`], 30 s) is sized
/// for control calls. `web_answer_tool` waits on a provider to search *and*
/// synthesize an answer, and `web_search_tool` can fall back across providers
/// one after another; both routinely exceeded 30 s in production and came back
/// as a bus timeout although the module was still working. 90 s stays under
/// the harness's own default tool deadline
/// ([`tinytools_std::timeout::DEFAULT_TIMEOUT_SECS`], 120 s), so the module's
/// answer, not the harness kill, decides the result. This is the floor; a
/// larger configured tool budget extends it (see [`execute_tool_timeout_for`]).
pub(super) const EXECUTE_TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

/// Headroom left between the bus deadline and the harness's tool deadline so
/// the module's own answer (or its bus error) reaches the harness first.
const EXECUTE_TOOL_HEADROOM_SECS: u64 = 30;

/// Bus deadline for `ExecuteTool` under a harness tool budget of
/// `tool_budget_secs`: the budget minus [`EXECUTE_TOOL_HEADROOM_SECS`], never
/// below [`EXECUTE_TOOL_TIMEOUT`]. A user who raises the action timeout (for a
/// slow provider) gets a search call that can use it; at the 120 s default
/// this is exactly the 90 s floor. Below the floor the harness deadline is the
/// tighter bound anyway.
pub(super) fn execute_tool_timeout_for(tool_budget_secs: u64) -> std::time::Duration {
    std::time::Duration::from_secs(
        tool_budget_secs
            .saturating_sub(EXECUTE_TOOL_HEADROOM_SECS)
            .max(EXECUTE_TOOL_TIMEOUT.as_secs()),
    )
}

/// Fingerprint of the configuration the loaded module holds: recorded when
/// this host builds its load-time configuration and after every
/// reinitialization. `None` when unknown — the module was admitted from the
/// module search path with no host configuration, or a reinitialization
/// failed partway — so the next call reconfigures it.
fn held() -> &'static std::sync::Mutex<Option<u64>> {
    static HELD: OnceLock<std::sync::Mutex<Option<u64>>> = OnceLock::new();
    HELD.get_or_init(|| std::sync::Mutex::new(None))
}

fn set_held(value: Option<u64>) {
    *held()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = value;
}

fn held_fingerprint() -> Option<u64> {
    *held()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Record `configuration` as what the module is about to be loaded with.
pub(super) fn record_load_configuration(configuration: &serde_json::Value) {
    set_held(Some(fingerprint(configuration)));
}

fn fingerprint(value: &serde_json::Value) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.to_string().hash(&mut hasher);
    hasher.finish()
}

/// The module proxy, with the module reconfigured to `config` first when it
/// holds anything else. Callers hold the module call lock
/// ([`with_module_lock`]) through the bus call that follows, so a call never
/// runs under another caller's configuration — two agents with different
/// pinned keys take turns, each reconfiguring the module for its own call.
async fn proxy(config: &Config) -> Result<tinybus::Proxy, String> {
    crate::modules::ops::ensure_loaded_within(
        config,
        MODULE_ID,
        Some(std::time::Duration::from_secs(8)),
    )
    .await
    .map_err(crate::modules::ops::load_error_message)?;
    let runtime = crate::modules::runtime::runtime()
        .await
        .map_err(|error| format!("search module bus unavailable: {error}"))?;
    let configuration =
        serde_json::to_value(module_config(config)).map_err(|error| error.to_string())?;
    let current = fingerprint(&configuration);
    if held_fingerprint() != Some(current) {
        tracing::debug!(
            host_pinned = config.search.is_host_pinned(),
            "[modules][search] module holds another configuration; reinitializing"
        );
        set_held(None);
        reinitialize(runtime, configuration).await?;
        set_held(Some(current));
    }
    runtime
        .proxy(names::INTERFACE, names::OBJECT_PATH)
        .map_err(|error| format!("search module proxy unavailable: {error}"))
}

/// Reinitialize, waiting out a module that is still finishing its previous
/// (re)initialization instead of failing the caller's search.
async fn reinitialize(
    runtime: &tinybus::module::ModuleRuntime,
    configuration: serde_json::Value,
) -> Result<(), String> {
    const ATTEMPTS: u32 = 40;
    for attempt in 1..=ATTEMPTS {
        match runtime
            .connection()
            .reinitialize_module(MODULE_ID, configuration.clone())
            .await
        {
            Ok(_) => return Ok(()),
            Err(error) if attempt < ATTEMPTS && error.to_string().contains("initializing") => {
                tracing::debug!(
                    attempt,
                    "[modules][search] module still initializing; retrying"
                );
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(error) => {
                return Err(format!(
                    "search module configuration refresh failed: {error}"
                ));
            }
        }
    }
    Err("search module configuration refresh failed: module never became ready".into())
}

pub(super) async fn with_module_lock<T, F, Fut>(operation: F) -> Result<T, String>
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

/// The config a search call runs under.
///
/// A config carrying host-pinned search credentials
/// ([`crate::config::SearchConfig::pin_host_credential`]) is authoritative:
/// it is one agent's in-memory config, its keys exist nowhere else, and a
/// reload of the shared `config_path` would drop them. Any other config is
/// reloaded so a settings or login change is honoured on the next call.
async fn current_config(ctx: &DispatchCtx, config: &Config) -> Result<Config, String> {
    if config.profile_providers.is_some() {
        let context = ctx
            .core()
            .ok_or_else(|| "profile search requires its authenticated context".to_owned())?;
        let current = context
            .embedder_config()
            .filter(|current| current.config_path == config.config_path)
            .filter(|_| context.profile().is_some())
            .ok_or_else(|| "profile search configuration belongs to another context".to_owned())?;
        return Ok(current.clone());
    }
    if config.search.is_host_pinned() {
        tracing::debug!(
            "[modules][search] host-pinned search credentials; using the agent's config"
        );
        return Ok(config.clone());
    }
    crate::config::ops::reload_config_from_paths(&config.config_path, &config.workspace_dir).await
}

/// Refresh a loaded module after a settings or credential change. An unloaded
/// module gets the current configuration during its first initialization.
pub async fn refresh_loaded(ctx: &DispatchCtx, config: &Config) -> Result<(), String> {
    if matches!(
        crate::modules::ops::state_of(MODULE_ID),
        crate::modules::types::ModuleState::Ready
    ) {
        let current = current_config(ctx, config).await?;
        with_module_lock(|| async {
            let _ = proxy(&current).await?;
            Ok(())
        })
        .await?;
    }
    Ok(())
}

pub async fn list_tools(ctx: &DispatchCtx, config: &Config) -> Result<ListToolsResponse, String> {
    let current = current_config(ctx, config).await?;
    with_module_lock(|| async {
        let mut listed: ListToolsResponse = proxy(&current)
            .await?
            .call(names::methods::LIST_TOOLS, ())
            .await
            .map_err(|error| format!("search ListTools failed: {error}"))?;
        if let Some(binding) = &current.profile_providers {
            binding.present(&mut listed.tools);
            let capabilities = crate::profiles::capabilities::ProfileCapabilities::in_ctx(ctx);
            listed.tools.retain(|tool| {
                capabilities
                    .as_ref()
                    .is_some_and(|capabilities| capabilities.admits(ctx, &tool.name))
            });
        }
        Ok(listed)
    })
    .await
}

/// The presentation and route boundary is checked before any module load or call.
fn canonical_request(
    ctx: &DispatchCtx,
    config: &Config,
    mut request: ExecuteToolRequest,
) -> Result<ExecuteToolRequest, String> {
    let Some(binding) = &config.profile_providers else {
        return Ok(request);
    };
    if !crate::profiles::capabilities::ProfileCapabilities::in_ctx(ctx)
        .is_some_and(|capabilities| capabilities.admits(ctx, &request.name))
    {
        return Err("search tool is outside the profile capability boundary".into());
    }
    let canonical = binding
        .canonical_role_name(&request.name)
        .ok_or_else(|| "search tool was not advertised by this profile".to_owned())?;
    if !super::configured_tool_specs(config)
        .iter()
        .any(|tool| tool.name == request.name)
    {
        return Err("search tool is unavailable in this profile".into());
    }
    let role = tinysearch_bus::role_for_tool(canonical)
        .ok_or_else(|| "search alias has no canonical role".to_owned())?;
    let module = module_config(config);
    if let Some(provider) = request
        .arguments
        .get("provider")
        .and_then(serde_json::Value::as_str)
    {
        if !module
            .presentation
            .roles
            .get(&role)
            .is_some_and(|order| order.iter().any(|allowed| allowed == provider))
        {
            return Err("search provider is outside the operator role policy".into());
        }
    }
    if request
        .arguments
        .get("depth")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|depth| depth != "standard")
        || [
            "api_key",
            "credential",
            "endpoint",
            "base_url",
            "route",
            "provider_url",
            "backend",
        ]
        .into_iter()
        .any(|key| request.arguments.get(key).is_some())
    {
        return Err("search arguments cannot override operator routes or capabilities".into());
    }
    request.name = canonical.to_owned();
    Ok(request)
}

pub async fn execute_tool(
    ctx: &DispatchCtx,
    config: &Config,
    request: ExecuteToolRequest,
) -> Result<ExecuteToolResponse, String> {
    let current = current_config(ctx, config).await?;
    let request = canonical_request(ctx, &current, request)?;
    let tool = request.name.clone();
    with_module_lock(|| async {
        let proxy = proxy(&current).await?;
        // Keys travel only in the private module configuration; a call
        // carries the model's arguments, which are not secrets. An ordinary
        // call also works with a developer override, which is never attested.
        call_execute_tool(proxy, request).await.map_err(|error| {
            tracing::debug!(tool = %tool, "[modules][search] ExecuteTool failed");
            format!("search ExecuteTool failed: {error}")
        })
    })
    .await
}

/// `ExecuteTool` through `proxy`, under a deadline derived from the effective
/// tool budget ([`execute_tool_timeout_for`]) rather than the bus default.
pub(super) async fn call_execute_tool<R: serde::de::DeserializeOwned>(
    proxy: tinybus::Proxy,
    request: ExecuteToolRequest,
) -> tinybus::Result<R> {
    let timeout = execute_tool_timeout_for(crate::tools::timeout::tool_execution_timeout_secs());
    tracing::debug!(
        tool = %request.name,
        timeout_secs = timeout.as_secs(),
        "[modules][search] ExecuteTool"
    );
    proxy
        .with_timeout(timeout)
        .call(names::methods::EXECUTE_TOOL, (request,))
        .await
}

#[cfg(test)]
#[path = "proxy_tests.rs"]
mod proxy_tests;
