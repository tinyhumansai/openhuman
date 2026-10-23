//! The immutable facts the host declares to the module for one sandbox scope.
//!
//! The module plans what may run from these facts, never from a request: the
//! deployment, which isolation switches are set, the filesystem and network
//! ceilings, and the executors the host verified it can drive. A scope's identity
//! is a digest of its facts, so two spawns with the same facts share a scope and a
//! spawn whose facts differ (another action dir, a jail that came up differently)
//! gets its own instead of widening an existing one.

use std::path::Path;

use sha2::{Digest, Sha256};
use tinysecurity_bus::{
    AccessTier, NetworkPolicy, Origin, PathAccess, PathGrant, PathPolicy, PathTrustedRoot,
    ResourceLimits2, SandboxDeployment2, SandboxExecutor2, SandboxScope2, SandboxSetting2,
    SandboxSource2, SourceTrust, SubjectScope,
};

use super::super::types::SandboxPolicy;
use crate::config::{Config, SandboxBackend};

/// The longest a plan stays valid: enough for one spawn, far below the module's
/// five-minute ceiling.
pub(crate) const PLAN_TTL_MS: u32 = 30_000;

/// Who wrote the text being run, which sets the isolation floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpawnSource {
    /// The agent's own shell tool under the operator's sandbox settings.
    Agent,
    /// Text the host did not author (a scheduled job, a hook): kernel isolation
    /// is mandatory, whatever the operator's switches say.
    Untrusted,
}

impl SpawnSource {
    pub(crate) const fn source_id(self) -> &'static str {
        match self {
            Self::Agent => "shell",
            Self::Untrusted => "scheduled-shell",
        }
    }

    /// The immutable identity of the executable source the module registers.
    pub(crate) const fn provenance(self) -> &'static str {
        match self {
            Self::Agent => "openhuman/shell/v1",
            Self::Untrusted => "openhuman/scheduled-shell/v1",
        }
    }

    /// Whether a best-effort process-level jail (macOS Seatbelt) may run this source's
    /// commands: the operator's own agent tools, never text the host did not author.
    /// The module enforces it again from the source's trust and the deployment.
    pub(crate) const fn accepts_process_isolation(self) -> bool {
        matches!(self, Self::Agent)
    }

    const fn trust(self) -> SourceTrust {
        match self {
            Self::Agent => SourceTrust::TrustedHost,
            Self::Untrusted => SourceTrust::UntrustedSkill,
        }
    }
}

/// Everything one scope's facts derive from.
pub(crate) struct FactsInput<'a> {
    pub(crate) config: &'a Config,
    pub(crate) policy: &'a SandboxPolicy,
    pub(crate) source: SpawnSource,
    pub(crate) subject: SubjectScope,
    pub(crate) saas: bool,
    /// The operator turned the sandbox off (environment, component or configuration).
    /// Sent whatever the deployment: the module ignores it under SaaS.
    pub(crate) operator_off: bool,
    /// Every executor the host verified, in the order the module should prefer them.
    pub(crate) executors: Vec<SandboxExecutor2>,
}

fn setting(on: bool) -> SandboxSetting2 {
    if on {
        SandboxSetting2::Enabled
    } else {
        SandboxSetting2::Disabled
    }
}

fn grant(path: &Path, write: bool) -> Option<PathGrant> {
    Some(PathGrant {
        path: path.to_str()?.to_owned(),
        write,
    })
}

/// System directories the native jail reads on its own (TinyBox's baseline). A
/// read-only grant under one is redundant, and the module refuses it anyway: a grant
/// may not cover a protected installation root.
const JAIL_BASELINE_ROOTS: &[&str] = &["/usr", "/bin", "/sbin", "/lib", "/lib64"];

/// Whether `path` is read by the jail's own baseline, so no grant is needed for it.
fn under_baseline(path: &Path) -> bool {
    JAIL_BASELINE_ROOTS
        .iter()
        .any(|root| path.starts_with(root))
}

/// The grants a spawn may ask for: its workspace read-write plus the policy's mounts.
/// `None` when a path cannot cross the bus as text.
pub(crate) fn grant_ceiling(policy: &SandboxPolicy) -> Option<Vec<PathGrant>> {
    let mut grants = vec![grant(&policy.workspace_root, true)?];
    for path in policy
        .read_only_mounts
        .iter()
        .filter(|path| !under_baseline(path))
    {
        grants.push(grant(path, false)?);
    }
    for path in &policy.read_write_mounts {
        grants.push(grant(path, true)?);
    }
    grants.sort_by(|a, b| a.path.cmp(&b.path).then(a.write.cmp(&b.write)));
    grants.dedup();
    Some(grants)
}

/// The network reach the policy allows: none, or the host's own network, which the
/// module plans for the operator's scope ([`operator_host_network`]) and otherwise only
/// after an exact human approval.
pub(crate) fn network_ceiling(policy: &SandboxPolicy) -> NetworkPolicy {
    if policy.allow_network {
        NetworkPolicy::Host
    } else {
        NetworkPolicy::None
    }
}

/// Whether the operator's own configuration grants the host network the policy allows,
/// so the module plans it without asking a human about each command. The policy is
/// the grant: this is true exactly when [`network_ceiling`] is the host network, and
/// the module still refuses it under SaaS and above the ceiling.
pub(crate) fn operator_host_network(policy: &SandboxPolicy) -> bool {
    network_ceiling(policy) == NetworkPolicy::Host
}

/// Whether the operator's configuration asks for a specific isolation backend.
fn configured(config: &Config) -> bool {
    matches!(
        config.security.sandbox.backend,
        SandboxBackend::Landlock | SandboxBackend::Docker
    )
}

/// The owner filesystem policy: the configured one, plus the roots the host already
/// chose for this spawn (its workspace and the mounts it resolved). The module still
/// refuses the protected floor and the core's internal state under every root.
fn paths(config: &Config, policy: &SandboxPolicy) -> PathPolicy {
    let mut paths = crate::modules::security::policy_translation::path_policy(config);
    // Reserved names protect internal files from the path tools (`.env`, `SOUL.md`...)
    // wherever they sit under the action dir. A jail's workspace grant is the whole
    // action dir by design, so under that rule no workspace could ever be granted.
    // The core's internal state under `workspace_dir` stays protected by its own
    // reservations, and a grant that overlaps `workspace_dir` is still refused.
    paths.reserved_names.clear();
    let roots = std::iter::once((&policy.workspace_root, PathAccess::ReadWrite))
        .chain(
            policy
                .read_only_mounts
                .iter()
                .filter(|path| !under_baseline(path))
                .map(|path| (path, PathAccess::ReadOnly)),
        )
        .chain(
            policy
                .read_write_mounts
                .iter()
                .map(|path| (path, PathAccess::ReadWrite)),
        );
    for (path, access) in roots {
        if !paths
            .trusted_roots
            .iter()
            .any(|root| &root.path == path && root.access == access)
        {
            paths.trusted_roots.push(PathTrustedRoot {
                path: path.clone(),
                access,
            });
        }
    }
    paths
}

/// The scope for `input`, or `None` when a path cannot be named to the module.
pub(crate) fn scope(input: &FactsInput<'_>) -> Option<SandboxScope2> {
    let FactsInput {
        config,
        policy,
        source,
        subject,
        saas,
        operator_off,
        executors,
    } = input;
    let mut facts = SandboxScope2 {
        approval_scope_id: None,
        scope_id: String::new(),
        subject: subject.clone(),
        origin: if *saas {
            Origin::Saas
        } else {
            Origin::Operator
        },
        tier: AccessTier::Full,
        deployment: if *saas {
            SandboxDeployment2::Saas
        } else {
            SandboxDeployment2::Local
        },
        environment: setting(!operator_off),
        // A scope is only planned for a spawn the agent or operator asked to sandbox.
        agent_mode: SandboxSetting2::Enabled,
        configured: setting(configured(config)),
        paths: paths(config, policy),
        grant_ceiling: grant_ceiling(policy)?,
        network_ceiling: network_ceiling(policy),
        operator_host_network: operator_host_network(policy),
        limits: ResourceLimits2::default(),
        sources: vec![SandboxSource2 {
            source_id: source.source_id().into(),
            provenance: source.provenance().into(),
            trust: source.trust(),
        }],
        executors: executors.clone(),
        proxy: None,
        max_ttl_ms: PLAN_TTL_MS,
    };
    facts.scope_id = scope_id(&facts)?;
    Some(facts)
}

/// A stable identity for the facts: a digest of everything but the identity itself.
fn scope_id(facts: &SandboxScope2) -> Option<String> {
    let mut canonical = serde_json::to_value(facts).ok()?;
    canonical.as_object_mut()?.remove("scope_id");
    canonical.sort_all_objects();
    let mut hash = Sha256::new();
    hash.update(b"openhuman/sandbox-scope/v1\0");
    hash.update(serde_json::to_vec(&canonical).ok()?);
    Some(format!("sbx-{:x}", hash.finalize()))
}

#[cfg(test)]
#[path = "facts_tests.rs"]
mod tests;
