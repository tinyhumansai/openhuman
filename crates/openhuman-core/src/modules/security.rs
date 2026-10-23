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
///
/// `trusted_host` is this host's own unique bus connection. The module accepts
/// privileged members (`RegisterPathPolicy`, `RegisterPolicy`, ...) only from
/// that exact peer, and the binding cannot be added by a later reinit, so it
/// must be present in the configuration the module is first loaded with. The
/// runtime is always started before a module is loaded.
pub fn module_config(config: &Config) -> ModuleConfig {
    ModuleConfig {
        trusted_host: super::runtime::started()
            .and_then(|runtime| runtime.connection().unique_name())
            .map(|name| name.to_string()),
        judge: policy_translation::judge_config(config),
        crypto: Some(production_crypto_config(config)),
        approvals: Some(approval_init_config()),
        audit: Some(audit_init_config()),
        sandbox: Some(sandbox_init_config()),
        ..ModuleConfig::default()
    }
}

/// Deadline of every approval callback the module makes into the host (store,
/// audit, prompt, human verification), in milliseconds.
const APPROVAL_CALLBACK_TIMEOUT_MS: u32 = 20_000;

/// The subject the module's init-time approval and audit configuration names.
///
/// The module requires at least one subject at initialization and freezes that
/// configuration for the life of the process, while the host serves an
/// unbounded set of subjects (profiles, agents, posture variants) that it
/// registers after init (`security::approval::native::scopes`). This subject
/// stands for none of them: nothing is ever evaluated under it.
fn bootstrap_subject() -> tinysecurity_bus::SubjectScope {
    tinysecurity_bus::SubjectScope {
        user: "openhuman".into(),
        workspace: "bootstrap".into(),
        agent: "bootstrap".into(),
    }
}

/// Init-only approval configuration. Identical on every load and refresh: the
/// module refuses a changed one.
pub(crate) fn approval_init_config() -> tinysecurity_bus::ApprovalConfig2 {
    tinysecurity_bus::ApprovalConfig2 {
        timeout_ms: APPROVAL_CALLBACK_TIMEOUT_MS,
        scopes: vec![tinysecurity_bus::ApprovalScope2 {
            scope_id: "0".repeat(32),
            subject: bootstrap_subject(),
            policy_ref: "openhuman/approval/bootstrap".into(),
            generation: 1,
            ceiling: tinysecurity_bus::AccessTier::Read,
            require_approval: true,
            auto_approve_all: false,
        }],
    }
}

/// Init-only audit configuration of the approval engine. Audit is forwarded to
/// the host's `AuditSink2` and is not marked required: a required audit would
/// refuse the path, command and egress permissions, which carry no audit
/// correlation. Approvals still fail closed when an event cannot be committed.
pub(crate) fn audit_init_config() -> tinysecurity_bus::AuditConfig2 {
    tinysecurity_bus::AuditConfig2 {
        scope: bootstrap_subject(),
        stream_id: "b".repeat(32),
        timeout_ms: APPROVAL_CALLBACK_TIMEOUT_MS,
        required: false,
        native_path: None,
        max_bytes: 1_000_000,
        max_archives: 1,
        journal_max_bytes: 1_000_000,
        journal_max_events: 10_000,
    }
}

/// Init-only sandbox configuration: no scope at all. The module holds at most 128
/// init scopes, each bound to the audit subject, while the host plans sandboxes for
/// an unbounded set of subjects, so every scope is registered at runtime
/// (`RegisterSandboxScope2`, `sandbox::spawn_plan`) under the subject's own audit
/// stream. Identical on every load and refresh: the module refuses a changed one.
pub(crate) fn sandbox_init_config() -> tinysecurity_bus::SandboxConfig2 {
    tinysecurity_bus::SandboxConfig2 { scopes: Vec::new() }
}

/// Production confidential crypto surface. Password crypt is on. The keyring and
/// pairing stay unset, so the module denies every keyring and pairing call and
/// creates no store: no production caller reads a module keyring yet (the host
/// `secret_storage` owns the stores callers use, in a different format), and an
/// unread store would be surface without a consumer. The consent callback is
/// bound to the same absence (`runtime::install_callbacks`). Enabling the
/// keyring belongs with the caller migration that reads it.
pub(crate) fn production_crypto_config(_config: &Config) -> tinysecurity_bus::CryptoConfig2 {
    tinysecurity_bus::CryptoConfig2 {
        max_payload_bytes: tinysecurity_bus::MAX_CRYPTO_PAYLOAD_BYTES,
        operations_per_minute: 600,
        keyring: None,
        pairing: None,
    }
}

#[derive(Default)]
struct ClientState {
    generation: Option<u64>,
    info: Option<PolicyInfo>,
    proxy: Option<tinybus::Proxy>,
    scopes: Vec<(tinysecurity_bus::PathPolicy, tinysecurity_bus::PathPolicyId)>,
    commands: Vec<(
        tinysecurity_bus::CommandPolicy,
        tinysecurity_bus::CommandPolicyId,
    )>,
    faulted: bool,
    /// The last [`proxy`] attempt failed while loading the module, before any
    /// peer answered. Nothing was attested, so nothing a peer said was
    /// distrusted: the next call may try again. Consumed by [`Self::latch`].
    load_failed: bool,
}

impl ClientState {
    /// Latch the client as faulted, unless the failure was only that the module
    /// had not finished loading (a slow first load must not poison the process).
    fn latch(&mut self) {
        if std::mem::take(&mut self.load_failed) {
            log::warn!("[security][client] module not loaded yet; client stays retryable");
            return;
        }
        self.faulted = true;
    }
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
    state.load_failed = false;
    if let Err(error) =
        super::ops::ensure_loaded_within(config, MODULE_ID, Some(LOAD_TIMEOUT)).await
    {
        state.load_failed = true;
        return Err(super::ops::load_error_message(error));
    }
    let runtime = match super::runtime::runtime().await {
        Ok(runtime) => runtime,
        Err(_) => {
            state.load_failed = true;
            return Err("TinySecurity bus unavailable".into());
        }
    };
    // Built only after the runtime is started: `trusted_host` is this host's own
    // bus connection, and a refresh sent without it would drop the privileged
    // peer binding and deny RegisterPathPolicy.
    let configuration = module_config(config);
    if configuration.trusted_host.is_none() {
        return Err("TinySecurity trusted host connection unavailable".into());
    }
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
    super::security_watch::ensure_watching(runtime, record.bus_name);
    Ok(proxy)
}

async fn require_attestation(
    proxy: &tinybus::Proxy,
    record: &super::ModuleRecord,
) -> Result<(), String> {
    let attestation = tokio::time::timeout(CALL_TIMEOUT, proxy.attestation())
        .await
        .map_err(|_| "TinySecurity attestation timed out")?
        .map_err(|_| "TinySecurity attestation failed")?;
    validate_attestation(attestation, record)
}

fn validate_attestation(
    attestation: Option<tinybus::attest::Attestation>,
    record: &super::ModuleRecord,
) -> Result<(), String> {
    let attestation = attestation.ok_or("TinySecurity recipient is not attested")?;
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

/// Make the module's active policy for the current tenant match `config`. The
/// subject comes from the authenticated tenant, never from the environment, and
/// SaaS work without a profile is refused.
pub(crate) async fn activate_for_config(
    config: &Config,
    mode: tinysecurity_bus::CommandExecutionMode,
    reason: &str,
) -> Result<scoped_client::Activation, String> {
    let tenant = crate::core::runtime::tenant::current_tenant()
        .map_err(|_| "TinySecurity policy requires an authenticated tenant scope")?;
    let subject = policy_translation::subject_scope(
        &tenant,
        &config.workspace_dir,
        crate::core::runtime::is_saas(),
    )?;
    let caller = VerifiedCaller::from_host(CallerContext {
        agent: subject.agent.clone(),
        thread: None,
        flow: None,
        call_id: format!("policy-activate:{reason}"),
        origin: tinysecurity_bus::Origin::Operator,
        tier: tinysecurity_bus::AccessTier::Full,
    })?;
    scoped_client::activate(config, caller, subject, mode).await
}

/// Fire-and-forget activation used by the live-policy reinit driver. A refused
/// or review-pending activation leaves the previous module policy in force and
/// is logged; it never changes what the host itself enforces.
pub(crate) fn spawn_activation(config: Config, generation: u64) {
    if !config.modules.enabled {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        log::debug!("[security][policy] no runtime; skipping activation generation={generation}");
        return;
    };
    handle.spawn(async move {
        let reason = format!("generation-{generation}");
        match activate_for_config(&config, tinysecurity_bus::CommandExecutionMode::HarnessGated, &reason).await {
            Ok(scoped_client::Activation::Active(active)) => log::debug!(
                "[security][policy] module policy_generation={} active for host generation={generation}",
                active.policy_generation
            ),
            Ok(scoped_client::Activation::ReviewRequired { widenings, .. }) => log::warn!(
                "[security][policy] host generation={generation} widens authority ({} change(s)); module keeps the previous policy until a human approves",
                widenings.len()
            ),
            Err(error) => log::warn!(
                "[security][policy] activation for host generation={generation} failed: {error}"
            ),
        }
    });
}

/// The attested module's bus proxy for the approval engine, loading the module
/// first. Calls on it carry no host-side fault latching: an approval the module
/// refuses is that request's answer, not a failure of the channel.
pub(crate) async fn approval_proxy(config: &Config) -> Result<tinybus::Proxy, String> {
    scoped_client::module_proxy(config).await
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
        state.latch();
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
        state.latch();
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
        state.latch();
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
        state.latch();
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
        state.latch();
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
        state.latch();
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
    #[cfg(test)]
    if let Some(result) = path_stub::validate_host_path(&policy, path, parent) {
        return result;
    }

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
        state.latch();
    }
    result
}

async fn register_cached_scope(
    state: &mut ClientState,
    proxy: &tinybus::Proxy,
    policy: tinysecurity_bus::PathPolicy,
) -> Result<tinysecurity_bus::PathPolicyId, String> {
    if let Some(id) = lru_lookup(&mut state.scopes, &policy) {
        return Ok(id);
    }
    let id = call_register_path_policy(proxy, policy.clone()).await?;
    lru_insert(&mut state.scopes, policy, id.clone());
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

/// Registered scopes the host retains per cache. A full cache forgets its
/// least recently used scope (registering an equal policy again is idempotent
/// in the module) instead of refusing, so a stream of distinct per-run scopes
/// never disables the client.
pub(crate) const MAX_SCOPES: usize = 1024;

/// The id for `policy`, marking it most recently used.
pub(crate) fn lru_lookup<P: PartialEq, I: Clone>(cache: &mut Vec<(P, I)>, policy: &P) -> Option<I> {
    let at = cache
        .iter()
        .position(|(registered, _)| registered == policy)?;
    let entry = cache.remove(at);
    let id = entry.1.clone();
    cache.push(entry);
    Some(id)
}

/// Remember `policy`, evicting the least recently used scope at capacity.
pub(crate) fn lru_insert<P, I>(cache: &mut Vec<(P, I)>, policy: P, id: I) {
    if cache.len() >= MAX_SCOPES {
        cache.remove(0);
    }
    cache.push((policy, id));
}

pub(super) async fn call_register_path_policy(
    proxy: &tinybus::Proxy,
    policy: tinysecurity_bus::PathPolicy,
) -> Result<tinysecurity_bus::PathPolicyId, String> {
    let id: tinysecurity_bus::PathPolicyId = proxy
        .call_confidential(names::methods::REGISTER_PATH_POLICY, (policy,))
        .await
        .map_err(|error| format!("TinySecurity RegisterPathPolicy failed: {error}"))?;
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
pub(crate) mod native_tests;

#[cfg(test)]
#[path = "security_path_stub.rs"]
pub(crate) mod path_stub;

#[cfg(test)]
#[path = "security_production_tests.rs"]
mod production_tests;

#[cfg(test)]
#[path = "security_gap_native_tests.rs"]
mod gap_native_tests;

#[cfg(test)]
#[path = "security_tenant_tests.rs"]
mod tenant_tests;

/// Policies bound to the released archive, for the ignored production-path
/// tests of the callers that moved onto the module.
#[cfg(test)]
#[path = "security_released_tests.rs"]
pub(crate) mod released_support;

#[path = "security_config.rs"]
mod config;

#[path = "security_policy.rs"]
pub(crate) mod policy_translation;

#[path = "security_scoped.rs"]
pub(crate) mod scoped_client;

#[path = "security_sync.rs"]
pub(crate) mod sync_seam;

#[path = "security_text.rs"]
pub(crate) mod text;

#[path = "security_egress.rs"]
pub(crate) mod egress;

#[path = "security_command.rs"]
pub(crate) mod command;

#[path = "security_gap.rs"]
pub(crate) mod gap;

#[path = "security_crypto.rs"]
pub(crate) mod crypto;

#[path = "security_sandbox.rs"]
pub(crate) mod sandbox;

#[path = "security_audit.rs"]
pub(crate) mod audit;

#[path = "security_client_types.rs"]
mod client_types;
pub(crate) use client_types::{CallError, VerifiedCaller};
