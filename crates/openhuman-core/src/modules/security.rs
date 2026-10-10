//! Trusted host adapter for the separately loaded TinySecurity policy module.
//!
//! Only the bus contract is linked. A missing, untrusted, timed-out or malformed
//! module response is an error, never permission to execute. The bootstrap
//! engine has a narrow diagnostic surface; acting path policy uses immutable
//! module-owned scopes rather than the process-global bootstrap policy.

use std::{sync::OnceLock, time::Duration};

use tinysecurity_bus::{
    names, CallerContext, Check, CheckRequest, CheckResponse, Decision, EvaluateRequest,
    ModuleConfig, PolicyInfo, ToolCall,
};

use crate::config::Config;

pub const MODULE_ID: &str = "tinysecurity";
const CALL_TIMEOUT: Duration = Duration::from_secs(2);
const LOAD_TIMEOUT: Duration = Duration::from_secs(8);

/// Bootstrap configuration delivered privately at SDK initialization. Acting
/// policy is bound separately to immutable scopes, never the process-global
/// bootstrap configuration. No OpenHuman enabled-policy settings are dropped
/// into this configuration as if the bootstrap engine could enforce them.
pub fn module_config(_config: &Config) -> ModuleConfig {
    ModuleConfig::default()
}

/// Caller identity supplied exclusively by trusted host dispatch. This wrapper
/// intentionally has no Deserialize implementation or model-facing controller.
pub(crate) struct VerifiedCaller(CallerContext);

impl VerifiedCaller {
    /// Wrap already authenticated identity and authority, never model arguments.
    pub(crate) fn from_host(context: CallerContext) -> Result<Self, String> {
        if context.agent.trim().is_empty() || context.call_id.trim().is_empty() {
            return Err("TinySecurity caller identity is missing".into());
        }
        Ok(Self(context))
    }
}

#[derive(Default)]
struct ClientState {
    generation: Option<u64>,
    info: Option<PolicyInfo>,
    proxy: Option<tinybus::Proxy>,
    scopes: Vec<(tinysecurity_bus::PathPolicy, tinysecurity_bus::PathPolicyId)>,
    faulted: bool,
}

fn state() -> &'static tokio::sync::Mutex<ClientState> {
    static STATE: OnceLock<tokio::sync::Mutex<ClientState>> = OnceLock::new();
    STATE.get_or_init(|| tokio::sync::Mutex::new(ClientState::default()))
}

fn require_enabled(config: &Config) -> Result<(), String> {
    if !config.modules.enabled {
        return Err("TinySecurity modules are disabled".into());
    }
    Ok(())
}

fn pinned(record: &super::ModuleRecord, digest: &str) -> bool {
    record
        .assets
        .iter()
        .any(|asset| asset.sha256.eq_ignore_ascii_case(digest))
}

async fn proxy(config: &Config, state: &mut ClientState) -> Result<tinybus::Proxy, String> {
    require_enabled(config)?;
    if state.faulted {
        return Err("TinySecurity client faulted; restart required".into());
    }
    if let Some(proxy) = &state.proxy {
        return Ok(proxy.clone());
    }
    let configuration = module_config(config);
    super::ops::ensure_loaded_within(config, MODULE_ID, Some(LOAD_TIMEOUT))
        .await
        .map_err(super::ops::LoadError::into_message)?;
    let runtime = super::host::runtime()
        .await
        .map_err(|_| "TinySecurity bus unavailable")?;
    let record = super::registry::find(MODULE_ID).ok_or("TinySecurity registry entry missing")?;
    let proxy = runtime
        .proxy(names::INTERFACE, names::OBJECT_PATH)
        .map_err(|_| "TinySecurity proxy unavailable")?
        .with_timeout(CALL_TIMEOUT);
    require_attestation(&proxy, record).await?;
    let mut info: PolicyInfo = proxy
        .call(names::methods::POLICY_INFO, ())
        .await
        .map_err(|_| "TinySecurity PolicyInfo failed")?;
    // Init configuration is delivered privately by the loader. An eager load
    // may belong to a different agent, so compare the actual effective policy
    // even on the first call. Never rely on a process-global fingerprint alone.
    if info.policy != configuration.policy {
        tokio::time::timeout(
            CALL_TIMEOUT,
            runtime.connection().reinitialize_module(
                "tinysecurity-module",
                serde_json::to_value(&configuration)
                    .map_err(|_| "TinySecurity config serialization failed")?,
            ),
        )
        .await
        .map_err(|_| "TinySecurity configuration refresh timed out")?
        .map_err(|_| "TinySecurity configuration refresh failed")?;
        info = proxy
            .call(names::methods::POLICY_INFO, ())
            .await
            .map_err(|_| "TinySecurity PolicyInfo failed")?;
    }
    validate_info(&info, &configuration)?;
    state.generation = Some(info.generation);
    state.info = Some(info);
    state.proxy = Some(proxy.clone());
    Ok(proxy)
}

async fn require_attestation(
    proxy: &tinybus::Proxy,
    record: &super::ModuleRecord,
) -> Result<(), String> {
    let attestation = tokio::time::timeout(CALL_TIMEOUT, proxy.attestation())
        .await
        .map_err(|_| "TinySecurity attestation timed out")?
        .map_err(|_| "TinySecurity attestation failed")?
        .ok_or("TinySecurity recipient is not attested")?;
    if attestation.name.as_str() != record.bus_name || !pinned(record, &attestation.sha256) {
        return Err("TinySecurity recipient does not match a pinned release".into());
    }
    Ok(())
}

fn validate_info(info: &PolicyInfo, config: &ModuleConfig) -> Result<(), String> {
    if !tinysecurity_bus::is_compatible(info.contract_version)
        || info.generation == 0
        || info.policy != config.policy
        || names::METHODS.iter().any(|method| {
            !info
                .capabilities
                .iter()
                .any(|available| available == method)
        })
    {
        return Err("TinySecurity effective policy or contract mismatch".into());
    }
    Ok(())
}

/// Read effective policy from the attested module, for authenticated host UI.
pub async fn policy_info(config: &Config) -> Result<PolicyInfo, String> {
    require_enabled(config)?;
    let mut state = state().lock().await;
    let result = async {
        proxy(config, &mut state).await?;
        state
            .info
            .clone()
            .ok_or_else(|| "TinySecurity policy unavailable".into())
    }
    .await;
    if result.is_err() {
        state.faulted = true;
    }
    result
}

/// Evaluate original arguments with complete host-derived effects. Errors deny
/// execution; no in-process fallback or automatic retry is performed.
pub(crate) async fn evaluate(
    config: &Config,
    caller: VerifiedCaller,
    call: ToolCall,
) -> Result<Decision, String> {
    require_enabled(config)?;
    let mut state = state().lock().await;
    let result = async {
        let proxy = proxy(config, &mut state).await?;
        let decision: Decision = proxy
            .call_confidential(
                names::methods::EVALUATE,
                (EvaluateRequest {
                    caller: caller.0,
                    call,
                },),
            )
            .await
            .map_err(|_| "TinySecurity Evaluate failed")?;
        validate_decision(&decision, state.generation)?;
        Ok(decision)
    }
    .await;
    if result.is_err() {
        state.faulted = true;
    }
    result
}

/// Batch concrete in-tool effects under one accepted policy generation.
pub(crate) async fn check(
    config: &Config,
    caller: VerifiedCaller,
    checks: Vec<Check>,
) -> Result<CheckResponse, String> {
    require_enabled(config)?;
    let mut state = state().lock().await;
    let result = async {
        let proxy = proxy(config, &mut state).await?;
        let count = checks.len();
        let response: CheckResponse = proxy
            .call_confidential(
                names::methods::CHECK,
                (CheckRequest {
                    caller: caller.0,
                    checks,
                },),
            )
            .await
            .map_err(|_| "TinySecurity Check failed")?;
        if response.decisions.len() != count || Some(response.generation) != state.generation {
            return Err("TinySecurity batch response mismatch".into());
        }
        for decision in &response.decisions {
            validate_decision(decision, state.generation)?;
        }
        Ok(response)
    }
    .await;
    if result.is_err() {
        state.faulted = true;
    }
    result
}

fn validate_decision(decision: &Decision, generation: Option<u64>) -> Result<(), String> {
    if Some(decision.generation) != generation || decision.generation == 0 || decision.cacheable {
        return Err("TinySecurity decision generation or caching mismatch".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "security_tests.rs"]
mod tests;

/// Register one immutable acting policy on the attested native module. Host
/// paths travel confidentially and never become a public controller payload.
pub(crate) async fn register_path_policy(
    config: &Config,
    policy: tinysecurity_bus::PathPolicy,
) -> Result<tinysecurity_bus::PathPolicyId, String> {
    require_enabled(config)?;
    let mut state = state().lock().await;
    let result = async {
        let proxy = proxy(config, &mut state).await?;
        register_cached_scope(&mut state, &proxy, policy).await
    }
    .await;
    if result.is_err() {
        state.faulted = true;
    }
    result
}

/// Resolve and authorize an existing path under a previously registered scope.
pub(crate) async fn validate_path(
    config: &Config,
    policy_id: tinysecurity_bus::PathPolicyId,
    path: String,
) -> Result<tinysecurity_bus::PathValidationResult, String> {
    validate_native_path(config, policy_id, path, names::methods::VALIDATE_PATH).await
}

/// Resolve and authorize the existing parent of a proposed file destination.
pub(crate) async fn validate_parent(
    config: &Config,
    policy_id: tinysecurity_bus::PathPolicyId,
    path: String,
) -> Result<tinysecurity_bus::PathValidationResult, String> {
    validate_native_path(config, policy_id, path, names::methods::VALIDATE_PARENT).await
}

async fn validate_native_path(
    config: &Config,
    policy_id: tinysecurity_bus::PathPolicyId,
    path: String,
    member: &str,
) -> Result<tinysecurity_bus::PathValidationResult, String> {
    require_enabled(config)?;
    let mut state = state().lock().await;
    let result = async {
        let proxy = proxy(config, &mut state).await?;
        call_validate_path(&proxy, policy_id, path, member).await
    }
    .await;
    if result.is_err() {
        state.faulted = true;
    }
    result
}

/// Query one native path helper using host-derived query kind and scope.
pub(crate) async fn check_path(
    config: &Config,
    policy_id: tinysecurity_bus::PathPolicyId,
    path: std::path::PathBuf,
    kind: tinysecurity_bus::PathCheckKind,
) -> Result<bool, String> {
    require_enabled(config)?;
    let mut state = state().lock().await;
    let result = async {
        let proxy = proxy(config, &mut state).await?;
        proxy
            .call_confidential(
                names::methods::CHECK_PATH,
                (tinysecurity_bus::PathCheckRequest {
                    policy_id,
                    path,
                    kind,
                },),
            )
            .await
            .map_err(|_| "TinySecurity CheckPath failed".into())
    }
    .await;
    if result.is_err() {
        state.faulted = true;
    }
    result
}

/// Resolve a concrete acting operation under host-derived immutable policy.
/// Configuration comes from the current dispatch context; the caller cannot
/// substitute a module source, runtime, scope identity or bootstrap settings.
pub(crate) async fn validate_host_path(
    policy: tinysecurity_bus::PathPolicy,
    path: &str,
    parent: bool,
) -> Result<tinysecurity_bus::PathValidationResult, String> {
    let config =
        config::host_config(&policy.workspace_dir, crate::core::runtime::is_saas()).await?;
    require_enabled(&config)?;
    let mut state = state().lock().await;
    let result = async {
        let proxy = proxy(&config, &mut state).await?;
        call_host_path(&mut state, &proxy, policy, path, parent).await
    }
    .await;
    if result.is_err() {
        state.faulted = true;
    }
    result
}

async fn register_cached_scope(
    state: &mut ClientState,
    proxy: &tinybus::Proxy,
    policy: tinysecurity_bus::PathPolicy,
) -> Result<tinysecurity_bus::PathPolicyId, String> {
    if let Some((_, id)) = state
        .scopes
        .iter()
        .find(|(registered, _)| registered == &policy)
    {
        return Ok(id.clone());
    }
    if state.scopes.len() >= 1024 {
        return Err("TinySecurity host path scope limit reached".into());
    }
    let id = call_register_path_policy(proxy, policy.clone()).await?;
    state.scopes.push((policy, id.clone()));
    Ok(id)
}

/// Shared production/native-fixture path operation. Warmed clients with equal
/// immutable scopes use one bus call; first use adds one registration call.
/// No permission result or filesystem resolution is cached.
async fn call_host_path(
    state: &mut ClientState,
    proxy: &tinybus::Proxy,
    policy: tinysecurity_bus::PathPolicy,
    path: &str,
    parent: bool,
) -> Result<tinysecurity_bus::PathValidationResult, String> {
    let id = register_cached_scope(state, proxy, policy).await?;
    let member = if parent {
        names::methods::VALIDATE_PARENT
    } else {
        names::methods::VALIDATE_PATH
    };
    call_validate_path(proxy, id, path.to_owned(), member).await
}

async fn call_register_path_policy(
    proxy: &tinybus::Proxy,
    policy: tinysecurity_bus::PathPolicy,
) -> Result<tinysecurity_bus::PathPolicyId, String> {
    let id: tinysecurity_bus::PathPolicyId = proxy
        .call_confidential(names::methods::REGISTER_PATH_POLICY, (policy,))
        .await
        .map_err(|_| "TinySecurity RegisterPathPolicy failed")?;
    if id.0.is_empty() {
        return Err("TinySecurity returned an empty path scope".into());
    }
    Ok(id)
}

async fn call_validate_path(
    proxy: &tinybus::Proxy,
    policy_id: tinysecurity_bus::PathPolicyId,
    path: String,
    member: &str,
) -> Result<tinysecurity_bus::PathValidationResult, String> {
    let result: tinysecurity_bus::PathValidationResult = proxy
        .call_confidential(
            member,
            (tinysecurity_bus::PathValidationRequest { policy_id, path },),
        )
        .await
        .map_err(|_| "TinySecurity path validation failed")?;
    validate_path_response(&result)?;
    Ok(result)
}

fn validate_path_response(result: &tinysecurity_bus::PathValidationResult) -> Result<(), String> {
    if let tinysecurity_bus::PathValidationResult::Allowed { resolved } = result {
        if !resolved.is_absolute() {
            return Err("TinySecurity returned a relative resolved path".into());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "security_native_tests.rs"]
mod native_tests;

#[path = "security_config.rs"]
mod config;
