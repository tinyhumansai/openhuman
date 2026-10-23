//! Sandbox planning through the TinySecurity module (issue #7328, phase 5).
//!
//! Before the host launches a sandboxed command it asks the module what may run.
//! The module owns the decision (the isolation floor, executor selection,
//! filesystem and network ceilings, resource bounds, mandatory SaaS isolation);
//! the host supplies the facts, the concrete operation, the audit correlation, and
//! executes exactly the plan it gets back:
//!
//! 1. [`facts::scope`] turns the policy, the configuration and every executor TinyBox
//!    verified into immutable scope facts, including the operator's own sandbox
//!    setting. A scope's id digests its facts, so a spawn with other facts gets its own
//!    scope.
//! 2. [`registry`] makes the module hold that scope for the subject
//!    (`RegisterSandboxScope2`) on the subject's own audit stream.
//! 3. [`plan`] sends `Plan2` for the concrete [`SpawnOperation`] and maps the issued
//!    artifact through [`BoxPlan::from_artifact`]. The module picks the executor (the
//!    configured backend is only a requirement it is sent) and alone decides whether
//!    the operator's setting may skip the jail: it plans a run on the host for the
//!    trusted operator outside SaaS and refuses every other source.
//! 4. [`consume`] reserves the artifact once, immediately before the spawn
//!    (`ConsumeSandboxPlan2`); a second use, or any change to the operation, is refused.
//!
//! Every failure is a refusal: a module fault, a denial, a malformed answer and an
//! artifact the host cannot map all mean the command does not run. Nothing falls
//! back to a host-side decision.

mod executors;
mod facts;
mod operation;
mod registry;

use tinysecurity_bus::{
    ConsumeSandboxPlanRequest2, NetworkPolicy, PathGrant, ResourceLimits2, SandboxArtifact2,
    SandboxBackend, SandboxDenial2, SandboxExecutor2, SandboxPlanOptions2, SandboxPlanRequest2,
    SubjectScope, Verdict,
};

pub(crate) use facts::SpawnSource;
pub(crate) use operation::SpawnOperation;

use super::plan::{BoxPlan, PlanMappingError};
use super::types::{SandboxBackendKind, SandboxPolicy};
use crate::config::Config;
use crate::modules::security::sandbox as client;
use crate::security::approval::native::audit_stream::AuditStream;

/// Why a spawn was not planned. Either way the command must not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PlanError {
    /// The module planned and refused.
    Refused(SandboxDenial2),
    /// The module could not be asked, answered something unusable, or the plan
    /// cannot be applied by the executors the host drives.
    Unavailable(String),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(denial) => write!(f, "the sandbox plan was refused ({denial:?})"),
            Self::Unavailable(reason) => write!(f, "the sandbox plan is unavailable: {reason}"),
        }
    }
}

impl std::error::Error for PlanError {}

fn unavailable(reason: impl Into<String>) -> PlanError {
    PlanError::Unavailable(reason.into())
}

/// A spawn the module planned and the host has not launched yet.
#[derive(Debug, Clone)]
pub(crate) struct PlannedSpawn {
    /// What TinyBox is asked to enforce: executor, grants, network and bounds.
    pub(crate) plan: BoxPlan,
    artifact: SandboxArtifact2,
    request: SandboxPlanRequest2,
    subject: SubjectScope,
    stream: AuditStream,
    proxy: tinybus::Proxy,
}

/// Where the module's bus proxy comes from.
#[derive(Clone, Copy)]
pub(crate) enum ModuleSource<'a> {
    /// The attested, pinned production module, loaded on demand.
    Production,
    /// A module a test loaded itself, with the executor facts it declares.
    Fixed(&'a FixedModule),
}

/// A security module and executor facts supplied by a test.
pub(crate) struct FixedModule {
    pub(crate) proxy: tinybus::Proxy,
    pub(crate) executors: Vec<SandboxExecutor2>,
}

impl ModuleSource<'_> {
    async fn proxy(self, config: &Config) -> Result<tinybus::Proxy, PlanError> {
        match self {
            Self::Production => client::production_proxy(config).await.map_err(unavailable),
            Self::Fixed(fixed) => Ok(fixed.proxy.clone()),
        }
    }
}

/// What a spawn is planned from.
#[derive(Clone, Copy)]
pub(crate) struct SpawnRequest<'a> {
    pub(crate) config: &'a Config,
    /// The resolved policy: which sandbox backend, workspace, mounts and network.
    pub(crate) policy: &'a SandboxPolicy,
    pub(crate) source: SpawnSource,
    pub(crate) operation: &'a SpawnOperation<'a>,
    /// The operator's own switch turned the sandbox off. It is a setting the scope
    /// carries; whether it skips the jail is the module's decision.
    pub(crate) operator_off: bool,
    /// Ask the module for a run on the host (needs `operator_off`). It plans one only
    /// for the trusted operator outside SaaS and refuses everything else (`floor`).
    pub(crate) host_run: bool,
    pub(crate) module: ModuleSource<'a>,
}

/// The requested resource bounds, from the policy's Docker overrides. Only a spawn the
/// policy requires a container for asks for them: a bound only a container enforces would
/// otherwise make every other executor ineligible.
fn limits(policy: &SandboxPolicy) -> ResourceLimits2 {
    let Some(overrides) = policy
        .docker_overrides
        .as_ref()
        .filter(|_| policy.backend == SandboxBackendKind::Docker)
    else {
        return ResourceLimits2::default();
    };
    ResourceLimits2 {
        cpu_time_ms: None,
        // Fractional CPUs become thousandths of a core, rounded up so a bound is
        // never silently loosened; absent bounds stay absent.
        cpu_millicores: overrides
            .cpu_limit
            .filter(|cpus| cpus.is_finite() && *cpus > 0.0)
            .map(|cpus| (cpus * 1000.0).ceil().min(f64::from(u32::MAX)) as u32),
        memory_bytes: overrides
            .memory_limit_mb
            .filter(|mb| *mb > 0)
            .and_then(|mb| mb.checked_mul(1024 * 1024)),
        pids_max: None,
        disk_bytes: None,
    }
}

/// The grants one spawn asks for: its workspace read-write plus the mounts the policy
/// names. A workspace-less spawn asks for no writable root at all.
fn grants(policy: &SandboxPolicy) -> Option<Vec<PathGrant>> {
    let mut grants = facts::grant_ceiling(policy)?;
    if !policy.mount_workspace {
        grants.retain(|grant| !grant.write);
    }
    Some(grants)
}

/// The executor preference, workspace and host-run options the policy asks for.
fn options(policy: &SandboxPolicy, host_run: bool) -> SandboxPlanOptions2 {
    SandboxPlanOptions2 {
        preferred_backend: match policy.backend {
            SandboxBackendKind::None => None,
            SandboxBackendKind::Local => Some(SandboxBackend::Jail),
            SandboxBackendKind::Docker => Some(SandboxBackend::Docker),
        },
        require_preferred_backend: policy.backend != SandboxBackendKind::None,
        no_workspace: !policy.mount_workspace,
        host_run,
    }
}

/// Every executor TinyBox verified on this host, in the order the module should prefer
/// them. A policy that requires one executor probes only that one. An executor whose
/// probe fails is declared unavailable; when every probe fails the spawn is refused.
async fn probe(
    config: &Config,
    policy: &SandboxPolicy,
) -> Result<Vec<SandboxExecutor2>, PlanError> {
    let jail = || async {
        crate::modules::tinybox::jail_status(config)
            .await
            .map(|status| executors::jail(&status))
            .map_err(|error| format!("jail status: {error}"))
    };
    let docker = || async {
        super::docker::is_docker_available(config)
            .await
            .map(executors::docker)
            .map_err(|error| format!("docker status: {error}"))
    };
    let (jail, docker) = match policy.backend {
        SandboxBackendKind::Local => (Some(jail().await), None),
        SandboxBackendKind::Docker => (None, Some(docker().await)),
        SandboxBackendKind::None => {
            let (jail, docker) = tokio::join!(jail(), docker());
            (Some(jail), Some(docker))
        }
    };
    let mut declared = Vec::new();
    let mut failure = None;
    if let Some(jail) = jail {
        match jail {
            Ok(executor) => declared.push(executor),
            Err(error) => {
                tracing::warn!(%error, "[sandbox:plan] the native jail could not be verified");
                declared.push(executors::jail_unavailable());
                failure.get_or_insert(error);
            }
        }
    }
    if let Some(docker) = docker {
        match docker {
            Ok(executor) => declared.push(executor),
            Err(error) => {
                tracing::debug!(%error, "[sandbox:plan] the container runtime could not be verified");
                declared.push(executors::docker(false));
                failure.get_or_insert(error);
            }
        }
    }
    match failure {
        Some(error) if declared.iter().all(|executor| !executor.available) => {
            Err(unavailable(error))
        }
        _ => Ok(declared),
    }
}

/// Plan one spawn through the module.
///
/// A spawn whose policy allows the network declares that allowance in its scope
/// (`operator_host_network`), so the module plans the host's own network without a
/// per-command human approval, up to the ceiling the policy set. Where the module still
/// wants an exact human approval (a SaaS scope, which no sandboxed spawn has a surface
/// to ask), the spawn is planned again with no network. The module only ever narrows
/// what the host asked for: the command never gets more than the policy allowed, and
/// one that needs the network fails inside the sandbox instead of running with it.
pub(crate) async fn plan(request: SpawnRequest<'_>) -> Result<PlannedSpawn, PlanError> {
    if request.host_run {
        return plan_with(&request, NetworkPolicy::Host).await;
    }
    let asked = facts::network_ceiling(request.policy);
    match plan_with(&request, asked.clone()).await {
        Err(PlanError::Refused(SandboxDenial2::Approval)) if asked == NetworkPolicy::Host => {
            tracing::warn!(
                "[sandbox:plan] host network needs a human approval; planning without network"
            );
            plan_with(&request, NetworkPolicy::None).await
        }
        other => other,
    }
}

async fn plan_with(
    request: &SpawnRequest<'_>,
    network: NetworkPolicy,
) -> Result<PlannedSpawn, PlanError> {
    let SpawnRequest {
        config,
        policy,
        source,
        operation,
        operator_off,
        host_run,
        module,
    } = *request;
    let saas = crate::core::runtime::is_saas();
    let tenant = crate::core::runtime::tenant::current_tenant()
        .map_err(|_| unavailable("no tenant scope"))?;
    let subject = crate::modules::security::policy_translation::subject_scope(
        &tenant,
        &config.workspace_dir,
        saas,
    )
    .map_err(unavailable)?;
    let proxy = module.proxy(config).await?;
    // A run on the host confines nothing, so no executor is verified for it.
    let executors = match module {
        _ if host_run => Vec::new(),
        ModuleSource::Production => probe(config, policy).await?,
        ModuleSource::Fixed(fixed) => fixed.executors.clone(),
    };
    let scope = facts::scope(&facts::FactsInput {
        config,
        policy,
        source,
        subject: subject.clone(),
        saas,
        operator_off,
        executors,
    })
    .ok_or_else(|| unavailable("a sandbox path is not valid text"))?;
    let stream = AuditStream::open(config, subject.clone())
        .await
        .map_err(unavailable)?;
    registry::ensure(&proxy, &scope, stream.stream_id()).await?;

    let operation_json = operation
        .canonical()
        .ok_or_else(|| unavailable("the working directory is not valid text"))?;
    // A run on the host claims no confinement: no grants and no bounds.
    let (grants, limits) = if host_run {
        (Vec::new(), ResourceLimits2::default())
    } else {
        let grants =
            grants(policy).ok_or_else(|| unavailable("a sandbox path is not valid text"))?;
        (grants, limits(policy))
    };
    let ordered = stream.lock().await;
    let generation = client::generation(&proxy).await.map_err(unavailable)?;
    let audit = stream.correlation(generation).await.map_err(unavailable)?;
    let call_id = audit.event_id.clone();
    let planned = SandboxPlanRequest2 {
        host_approval: None,
        scope_id: scope.scope_id.clone(),
        subject: subject.clone(),
        source_id: source.source_id().into(),
        provenance: source.provenance().into(),
        call_id,
        operation_json,
        grants,
        network,
        limits,
        deny_subprocesses: false,
        accept_process_isolation: source.accepts_process_isolation(),
        options: options(policy, host_run),
        generation,
        ttl_ms: facts::PLAN_TTL_MS,
        audit,
    };
    let response = client::plan(&proxy, planned.clone()).await;
    drop(ordered);
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            registry::forget(&scope.scope_id).await;
            return Err(unavailable(error));
        }
    };
    if let Some(denial) = response.denial {
        if denial == SandboxDenial2::InvalidContext {
            // The module no longer holds the scope (a reload): register it again next time.
            registry::forget(&scope.scope_id).await;
        }
        tracing::warn!(?denial, "[sandbox:plan] the module refused the spawn");
        return Err(PlanError::Refused(denial));
    }
    let artifact = match (response.decision.verdict, response.artifact) {
        (Verdict::Allow, Some(artifact)) => artifact,
        _ => return Err(unavailable("the module returned no plan")),
    };
    if response.audit.is_none_or(|audit| !audit.committed) {
        return Err(unavailable("the plan was not audited"));
    }
    if artifact.scope_id != scope.scope_id || artifact.subject != subject {
        return Err(unavailable("the plan names another scope"));
    }
    let home = super::local_grants::grant_home(saas, dirs::home_dir);
    let box_plan = BoxPlan::from_artifact(&artifact, home.as_deref())
        .map_err(|error: PlanMappingError| unavailable(error.to_string()))?;
    tracing::debug!(
        executor = ?box_plan.executor,
        grants = artifact.grants.len(),
        "[sandbox:plan] the module planned the spawn"
    );
    Ok(PlannedSpawn {
        plan: box_plan,
        artifact,
        request: planned,
        subject,
        stream,
        proxy,
    })
}

impl PlannedSpawn {
    /// The policy the spawn runs under: the caller's policy narrowed to what the module
    /// planned. Mounts, network reach and container bounds come from the artifact, never
    /// from the policy that was asked.
    pub(crate) fn apply(&self, policy: &SandboxPolicy) -> SandboxPolicy {
        let mut applied = policy.clone();
        let workspace = policy.workspace_root.to_str();
        let paths = |list: &[String]| -> Vec<std::path::PathBuf> {
            list.iter()
                .filter(|path| Some(path.as_str()) != workspace)
                .map(std::path::PathBuf::from)
                .collect()
        };
        applied.read_only_mounts = paths(&self.plan.jail.read_only);
        applied.read_write_mounts = paths(&self.plan.jail.read_write);
        applied.allow_network = self.plan.jail.allow_net;
        applied.mount_workspace = self.plan.workspace;
        if let Some(overrides) = applied.docker_overrides.as_mut() {
            if !applied.allow_network {
                // The container's own network setting must follow the plan too, or a
                // narrowed plan would still start the container on a network.
                overrides.network = Some("none".to_owned());
            }
            // A bound the plan carries wins. A plan that names none (the module chose
            // the container, so the spawn asked for no bound) keeps the operator's own
            // configured bound: it can only be tighter than no bound at all.
            let limits = &self.artifact.limits;
            if let Some(bytes) = limits.memory_bytes {
                overrides.memory_limit_mb = Some(bytes.div_ceil(1024 * 1024));
            }
            if let Some(millis) = limits.cpu_millicores {
                overrides.cpu_limit = Some(f64::from(millis) / 1000.0);
            }
        }
        applied
    }
}

/// Reserve `planned` once, immediately before launching it.
pub(crate) async fn consume(planned: &PlannedSpawn) -> Result<(), PlanError> {
    let _ordered = planned.stream.lock().await;
    let generation = client::generation(&planned.proxy)
        .await
        .map_err(unavailable)?;
    let audit = planned
        .stream
        .correlation(generation)
        .await
        .map_err(unavailable)?;
    let response = client::consume(
        &planned.proxy,
        ConsumeSandboxPlanRequest2 {
            request: planned.request.clone(),
            artifact: planned.artifact.clone(),
            subject: planned.subject.clone(),
            audit,
        },
    )
    .await
    .map_err(unavailable)?;
    if response.permitted {
        return Ok(());
    }
    let denial = response.denial.unwrap_or(SandboxDenial2::InvalidContext);
    tracing::warn!(
        ?denial,
        "[sandbox:plan] the module refused to admit the spawn"
    );
    Err(PlanError::Refused(denial))
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "integration_tests.rs"]
mod integration_tests;
