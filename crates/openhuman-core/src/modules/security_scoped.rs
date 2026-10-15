//! Host client for the module's scoped policy: `RegisterPolicy`,
//! `SupersedePolicy`, `EvaluateScoped`, `CheckScoped` and `ScopedPolicyInfo`,
//! plus the host decision cache.
//!
//! The module owns the decision; this file owns only transport, the mapping of
//! a verified subject to its active policy id, and a small cache that stores a
//! decision only when the module marked it `cacheable`. Rates, approvals and DNS
//! outcomes are never cacheable (the module says so), so the cache cannot turn
//! them into a stale permission. Errors deny; nothing falls back to in-process
//! policy.

use std::collections::HashMap;

use sha2::{Digest, Sha256};
use tinysecurity_bus::{
    names, CommandExecutionMode, CommandPolicy, CommandPolicyId, Decision, EvaluateRequest,
    PathPolicyId, PolicyChanged, PolicyId, RegisteredPolicy, ScopedCheckRequest,
    ScopedCheckResponse, ScopedDecision, ScopedEvaluateRequest, ScopedPolicyInfo, SubjectScope,
    SupersedePolicyRequest, SupersedePolicyResponse, ToolCall, Widening,
};

use super::{policy_translation as translate, state, VerifiedCaller};
use crate::config::Config;

const CACHE_CAPACITY: usize = 4096;

/// Cache identity of one decision: the policy lineage generation that decided,
/// the canonical call fingerprint and the verified subject.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CacheKey {
    pub policy_generation: u64,
    pub fingerprint: String,
    pub scope: String,
}

/// Decisions the module explicitly allowed the host to reuse.
#[derive(Default)]
pub(crate) struct DecisionCache {
    module_generation: u64,
    entries: HashMap<CacheKey, Decision>,
}

impl DecisionCache {
    pub(crate) fn get(&self, key: &CacheKey) -> Option<Decision> {
        // Keyed on the call fingerprint + policy generation (`CacheKey`) and
        // additionally bound to the module generation that decided: an entry
        // from any other module generation is never served.
        self.entries
            .get(key)
            .filter(|decision| decision.generation == self.module_generation)
            .cloned()
    }

    /// Store `decision` only when the module marked it cacheable. Returns
    /// whether it was stored.
    pub(crate) fn put(&mut self, key: CacheKey, decision: &Decision) -> bool {
        if !decision.cacheable || decision.generation == 0 {
            return false;
        }
        self.observe(decision.generation);
        if self.entries.len() >= CACHE_CAPACITY {
            self.entries.clear();
        }
        self.entries.insert(key, decision.clone());
        true
    }

    /// Record the module generation seen on a response; any change flushes.
    pub(crate) fn observe(&mut self, module_generation: u64) {
        if self.module_generation != module_generation {
            if !self.entries.is_empty() {
                log::debug!(
                    "[security][cache] generation {} -> {} flushed {} decision(s)",
                    self.module_generation,
                    module_generation,
                    self.entries.len()
                );
            }
            self.entries.clear();
            self.module_generation = module_generation;
        }
    }

    /// `PolicyChanged` arrived: nothing decided under an older policy survives.
    pub(crate) fn invalidate(&mut self, event: &PolicyChanged) {
        self.module_generation = event.generation;
        self.entries.clear();
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// An activated policy of one verified subject.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ActivePolicy {
    pub policy_id: PolicyId,
    pub policy_generation: u64,
    pub subject: SubjectScope,
    /// Fingerprint of the complete translated registration.
    pub registration: String,
}

/// Outcome of activating the policy `Config` translates to.
#[derive(Debug)]
pub(crate) enum Activation {
    Active(ActivePolicy),
    /// A widening replacement: approve `review` under the old policy with
    /// `EvaluateApproval2`, then retry with the approved request.
    ReviewRequired {
        review: EvaluateRequest,
        widenings: Vec<Widening>,
    },
}

#[derive(Default)]
pub(crate) struct ScopedState {
    active: HashMap<String, ActivePolicy>,
    commands: HashMap<String, CommandPolicyId>,
    cache: DecisionCache,
}

pub(crate) fn scoped() -> &'static tokio::sync::Mutex<ScopedState> {
    static STATE: std::sync::OnceLock<tokio::sync::Mutex<ScopedState>> = std::sync::OnceLock::new();
    STATE.get_or_init(|| tokio::sync::Mutex::new(ScopedState::default()))
}

fn subject_key(subject: &SubjectScope) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}",
        subject.user, subject.workspace, subject.agent
    )
}

fn digest(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    hex::encode(hasher.finalize())
}

/// Host-side fingerprint of a scoped call, for cache lookup. Excludes the call
/// id, as the module's own fingerprint does.
pub(crate) fn call_fingerprint(
    policy: &PolicyId,
    caller: &tinysecurity_bus::CallerContext,
    call: &ToolCall,
) -> Result<String, String> {
    let mut caller = caller.clone();
    caller.call_id.clear();
    let caller = serde_json::to_vec(&caller).map_err(|_| "TinySecurity caller encoding failed")?;
    let call = serde_json::to_vec(call).map_err(|_| "TinySecurity call encoding failed")?;
    Ok(digest(&[policy.0.as_bytes(), &caller, &call]))
}

fn registration_fingerprint(
    path: &tinysecurity_bus::PathPolicy,
    command: &CommandPolicy,
    config: &Config,
    subject: &SubjectScope,
) -> Result<String, String> {
    let rules = translate::tool_rules(config).rules;
    let bytes = serde_json::to_vec(&(path, command, translate::settings(config), rules, subject))
        .map_err(|_| "TinySecurity policy encoding failed")?;
    Ok(digest(&[&bytes]))
}

fn validate_scoped(
    generation: u64,
    policy_generation: u64,
    fingerprint: &str,
    active: &ActivePolicy,
) -> Result<(), String> {
    if generation == 0 || policy_generation != active.policy_generation || fingerprint.len() != 64 {
        return Err("TinySecurity scoped response generation or fingerprint mismatch".into());
    }
    Ok(())
}

pub(super) async fn module_proxy(config: &Config) -> Result<tinybus::Proxy, String> {
    let mut state = state().lock().await;
    let result = super::proxy(config, &mut state).await;
    if result.is_err() {
        state.faulted = true;
    } else {
        super::text::arm_log_scrubber(config);
    }
    result
}

pub(super) async fn register_command_policy(
    proxy: &tinybus::Proxy,
    policy: CommandPolicy,
) -> Result<CommandPolicyId, String> {
    let id: CommandPolicyId = proxy
        .call_confidential(names::methods::REGISTER_COMMAND_POLICY, (policy,))
        .await
        .map_err(|_| "TinySecurity RegisterCommandPolicy failed")?;
    if id.0.is_empty() {
        return Err("TinySecurity returned an empty command scope".into());
    }
    Ok(id)
}

/// Make the policy `config` translates to the active policy of `subject`,
/// registering it the first time and superseding the previous one when the
/// translation changed. Unchanged settings reuse the active policy, so a
/// subject's command budget is never reset by an idempotent call.
pub(crate) async fn activate(
    config: &Config,
    caller: VerifiedCaller,
    subject: SubjectScope,
    mode: CommandExecutionMode,
) -> Result<Activation, String> {
    super::require_enabled(config)?;
    let proxy = module_proxy(config).await?;
    activate_on(&proxy, scoped(), config, caller, subject, mode).await
}

/// [`activate`] against an explicit proxy and state, for the host runtime and
/// for tests that stand up their own broker.
pub(crate) async fn activate_on(
    proxy: &tinybus::Proxy,
    state: &tokio::sync::Mutex<ScopedState>,
    config: &Config,
    caller: VerifiedCaller,
    subject: SubjectScope,
    mode: CommandExecutionMode,
) -> Result<Activation, String> {
    let path = translate::path_policy(config);
    let command = translate::command_policy(config, mode, dirs::home_dir());
    let registration = registration_fingerprint(&path, &command, config, &subject)?;
    let key = subject_key(&subject);
    {
        let state = state.lock().await;
        if let Some(active) = state.active.get(&key) {
            if active.registration == registration {
                return Ok(Activation::Active(active.clone()));
            }
        }
    }
    let path_id: PathPolicyId = super::call_register_path_policy(proxy, path).await?;
    let command_key = serde_json::to_string(&command).map_err(|_| "command encoding failed")?;
    let cached_command = state.lock().await.commands.get(&command_key).cloned();
    let command_id = match cached_command {
        Some(id) => id,
        None => {
            let id = register_command_policy(proxy, command).await?;
            state.lock().await.commands.insert(command_key, id.clone());
            id
        }
    };
    let next: RegisteredPolicy =
        translate::registered_policy(config, subject.clone(), path_id, command_id);
    let previous = state.lock().await.active.get(&key).cloned();
    let (policy_id, policy_generation) = match previous {
        None => {
            let id: PolicyId = proxy
                .call_confidential(names::methods::REGISTER_POLICY, (next,))
                .await
                .map_err(|_| "TinySecurity RegisterPolicy failed")?;
            if id.0.is_empty() {
                return Err("TinySecurity returned an empty policy id".into());
            }
            (id, 1)
        }
        Some(previous) => {
            let response: SupersedePolicyResponse = proxy
                .call_confidential(
                    names::methods::SUPERSEDE_POLICY,
                    (SupersedePolicyRequest {
                        caller: caller.0,
                        previous: previous.policy_id,
                        next,
                        approval: None,
                    },),
                )
                .await
                .map_err(|_| "TinySecurity SupersedePolicy failed")?;
            match response {
                SupersedePolicyResponse::Activated {
                    policy_id,
                    policy_generation,
                } => (policy_id, policy_generation),
                SupersedePolicyResponse::ReviewRequired { review, widenings } => {
                    log::info!(
                        "[security][policy] replacement widens authority ({} change(s)); review required",
                        widenings.len()
                    );
                    return Ok(Activation::ReviewRequired { review, widenings });
                }
            }
        }
    };
    let active = ActivePolicy {
        policy_id,
        policy_generation,
        subject,
        registration,
    };
    let mut state = state.lock().await;
    state.cache = DecisionCache::default();
    state.active.insert(key, active.clone());
    log::debug!(
        "[security][policy] activated policy_generation={}",
        active.policy_generation
    );
    Ok(Activation::Active(active))
}

/// Evaluate an original tool call under the subject's registered policy.
/// A cacheable decision is reused for an identical call at the same policy
/// generation; any other decision is fetched every time.
pub(crate) async fn evaluate_scoped(
    config: &Config,
    active: &ActivePolicy,
    caller: VerifiedCaller,
    call: ToolCall,
) -> Result<ScopedDecision, String> {
    super::require_enabled(config)?;
    let proxy = module_proxy(config).await?;
    evaluate_on(&proxy, scoped(), active, caller, call).await
}

/// [`evaluate_scoped`] against an explicit proxy and state.
pub(crate) async fn evaluate_on(
    proxy: &tinybus::Proxy,
    state: &tokio::sync::Mutex<ScopedState>,
    active: &ActivePolicy,
    caller: VerifiedCaller,
    call: ToolCall,
) -> Result<ScopedDecision, String> {
    let fingerprint = call_fingerprint(&active.policy_id, &caller.0, &call)?;
    let key = CacheKey {
        policy_generation: active.policy_generation,
        fingerprint: fingerprint.clone(),
        scope: subject_key(&active.subject),
    };
    if let Some(decision) = state.lock().await.cache.get(&key) {
        return Ok(ScopedDecision {
            decision,
            policy_generation: active.policy_generation,
            fingerprint,
        });
    }
    let response: ScopedDecision = proxy
        .call_confidential(
            names::methods::EVALUATE_SCOPED,
            (ScopedEvaluateRequest {
                policy_id: active.policy_id.clone(),
                caller: caller.0,
                call,
            },),
        )
        .await
        .map_err(|_| "TinySecurity EvaluateScoped failed")?;
    validate_scoped(
        response.decision.generation,
        response.policy_generation,
        &response.fingerprint,
        active,
    )?;
    state.lock().await.cache.put(key, &response.decision);
    Ok(response)
}

/// Ordered in-tool checks under the subject's registered policy. Never cached:
/// a batch can carry command checks, which reserve budget.
pub(crate) async fn check_scoped(
    config: &Config,
    active: &ActivePolicy,
    caller: VerifiedCaller,
    checks: Vec<tinysecurity_bus::Check>,
) -> Result<ScopedCheckResponse, String> {
    super::require_enabled(config)?;
    let proxy = module_proxy(config).await?;
    check_on(&proxy, active, caller, checks).await
}

/// [`check_scoped`] against an explicit proxy.
pub(crate) async fn check_on(
    proxy: &tinybus::Proxy,
    active: &ActivePolicy,
    caller: VerifiedCaller,
    checks: Vec<tinysecurity_bus::Check>,
) -> Result<ScopedCheckResponse, String> {
    let count = checks.len();
    let response: ScopedCheckResponse = proxy
        .call_confidential(
            names::methods::CHECK_SCOPED,
            (ScopedCheckRequest {
                policy_id: active.policy_id.clone(),
                caller: caller.0,
                checks,
            },),
        )
        .await
        .map_err(|_| "TinySecurity CheckScoped failed")?;
    let generation = response.decisions.first().map_or(0, |d| d.generation);
    validate_scoped(
        generation,
        response.policy_generation,
        &response.fingerprint,
        active,
    )?;
    if response.decisions.len() != count
        || response
            .decisions
            .iter()
            .any(|d| d.generation != generation)
    {
        return Err("TinySecurity scoped batch response mismatch".into());
    }
    Ok(response)
}

/// The module's complete effective settings for one registered policy.
pub(crate) async fn scoped_policy_info(
    config: &Config,
    id: PolicyId,
) -> Result<ScopedPolicyInfo, String> {
    super::require_enabled(config)?;
    let proxy = module_proxy(config).await?;
    proxy
        .call_confidential(names::methods::SCOPED_POLICY_INFO, (id,))
        .await
        .map_err(|_| "TinySecurity ScopedPolicyInfo failed".into())
}

/// Entry point for a `PolicyChanged` subscriber: drop every cached decision.
pub(crate) async fn policy_changed(event: &PolicyChanged) {
    super::text::flush_cache();
    scoped().lock().await.cache.invalidate(event);
}

/// A lagged or lost signal stream cannot say what it missed: drop everything.
pub(crate) async fn flush_all() {
    super::text::flush_cache();
    let mut scoped = scoped().lock().await;
    scoped.cache.observe(0);
    scoped.cache.entries.clear();
}

/// Audit correlation the trusted host allocates once per audited operation.
pub(crate) use tinysecurity_bus::AuditCorrelation2 as AuditCorrelation;
use tinysecurity_bus::{AuditedRequest2, AuditedResponse2};

/// Audited counterpart of [`evaluate_on`] for deployments where audit is
/// required: the plain scoped call denies under required audit. The decision is
/// never cached (every use needs its own durable event) and the module's audit
/// evidence is returned alongside it.
pub(crate) async fn evaluate_audited_on(
    proxy: &tinybus::Proxy,
    active: &ActivePolicy,
    caller: VerifiedCaller,
    call: ToolCall,
    audit: AuditCorrelation,
) -> Result<AuditedResponse2<ScopedDecision>, String> {
    let response: AuditedResponse2<ScopedDecision> = proxy
        .call_confidential(
            names::methods::EVALUATE_SCOPED_AUDITED,
            (AuditedRequest2 {
                request: ScopedEvaluateRequest {
                    policy_id: active.policy_id.clone(),
                    caller: caller.0,
                    call,
                },
                audit,
            },),
        )
        .await
        .map_err(|_| "TinySecurity EvaluateScopedAudited failed")?;
    let d = &response.decision;
    validate_scoped(
        d.decision.generation,
        d.policy_generation,
        &d.fingerprint,
        active,
    )?;
    if d.decision.cacheable {
        return Err("TinySecurity audited decision claimed to be cacheable".into());
    }
    Ok(response)
}

/// Audited counterpart of [`check_on`]; see [`evaluate_audited_on`].
pub(crate) async fn check_audited_on(
    proxy: &tinybus::Proxy,
    active: &ActivePolicy,
    caller: VerifiedCaller,
    checks: Vec<tinysecurity_bus::Check>,
    audit: AuditCorrelation,
) -> Result<AuditedResponse2<ScopedCheckResponse>, String> {
    let count = checks.len();
    let response: AuditedResponse2<ScopedCheckResponse> = proxy
        .call_confidential(
            names::methods::CHECK_SCOPED_AUDITED,
            (AuditedRequest2 {
                request: ScopedCheckRequest {
                    policy_id: active.policy_id.clone(),
                    caller: caller.0,
                    checks,
                },
                audit,
            },),
        )
        .await
        .map_err(|_| "TinySecurity CheckScopedAudited failed")?;
    let d = &response.decision;
    let generation = d.decisions.first().map_or(0, |x| x.generation);
    validate_scoped(generation, d.policy_generation, &d.fingerprint, active)?;
    if d.decisions.len() != count
        || d.decisions
            .iter()
            .any(|x| x.generation != generation || x.cacheable)
    {
        return Err("TinySecurity audited batch response mismatch".into());
    }
    Ok(response)
}

/// Audited evaluation through the live module client.
pub(crate) async fn evaluate_scoped_audited(
    config: &Config,
    active: &ActivePolicy,
    caller: VerifiedCaller,
    call: ToolCall,
    audit: AuditCorrelation,
) -> Result<AuditedResponse2<ScopedDecision>, String> {
    super::require_enabled(config)?;
    let proxy = module_proxy(config).await?;
    evaluate_audited_on(&proxy, active, caller, call, audit).await
}

/// Audited ordered checks through the live module client.
pub(crate) async fn check_scoped_audited(
    config: &Config,
    active: &ActivePolicy,
    caller: VerifiedCaller,
    checks: Vec<tinysecurity_bus::Check>,
    audit: AuditCorrelation,
) -> Result<AuditedResponse2<ScopedCheckResponse>, String> {
    super::require_enabled(config)?;
    let proxy = module_proxy(config).await?;
    check_audited_on(&proxy, active, caller, checks, audit).await
}

#[cfg(test)]
#[path = "security_scoped_tests.rs"]
mod tests;
