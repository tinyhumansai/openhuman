//! The policy a spawn asks the security module to plan: where it works, what it mounts and
//! which executor the operator's configuration names.
//!
//! Nothing here chooses an executor. The configuration's backend becomes a requirement
//! carried in the request ([`SandboxPolicy::backend`]); the module picks among the
//! executors the host verified, refuses what the deployment forbids and decides whether
//! the sandbox may be skipped at all (`spawn_plan/`).

use std::path::Path;

use super::ops::SANDBOX_ENV_PASSTHROUGH;
use super::types::{DockerOverrides, SandboxBackendKind, SandboxPolicy};
use crate::config::{Config, RuntimeConfig, SandboxBackend, SandboxConfig};

/// The executor `sandbox` and `runtime` name, if any. `remote` marks a channel, cron or
/// remote session, which has always asked for a container.
fn named_backend(
    sandbox: &SandboxConfig,
    runtime: &RuntimeConfig,
    remote: bool,
) -> SandboxBackendKind {
    match sandbox.backend {
        SandboxBackend::Landlock => SandboxBackendKind::Local,
        SandboxBackend::Docker => SandboxBackendKind::Docker,
        SandboxBackend::Auto | SandboxBackend::None => {
            // `[runtime] kind = "docker"` is the legacy spelling of an operator-required
            // container.
            if remote || runtime.kind == "docker" {
                SandboxBackendKind::Docker
            } else {
                SandboxBackendKind::None
            }
        }
    }
}

/// The policy for one spawn from the operator's configuration.
///
/// `state_dir` is the core's internal `workspace_dir`; host-side scratch such as output
/// capture goes there instead of into `action_dir`. The jail's toolchain and git grants
/// are resolved by TinyBox when the spawn is planned (`sandbox::local_grants`), so the
/// policy starts without mounts.
pub fn spawn_policy(
    action_dir: &Path,
    state_dir: &Path,
    sandbox: &SandboxConfig,
    runtime: &RuntimeConfig,
    remote: bool,
) -> SandboxPolicy {
    let backend = named_backend(sandbox, runtime, remote);
    let docker = &runtime.docker;
    tracing::debug!(
        backend = ?backend,
        remote,
        action_dir = %action_dir.display(),
        "[sandbox] spawn policy"
    );
    SandboxPolicy {
        backend,
        workspace_root: action_dir.to_path_buf(),
        state_dir: state_dir.to_path_buf(),
        read_only_mounts: Vec::new(),
        read_write_mounts: Vec::new(),
        allow_network: !remote,
        env_passthrough: SANDBOX_ENV_PASSTHROUGH
            .iter()
            .map(|s| s.to_string())
            .collect(),
        // Carried whichever executor the module picks; only a container reads it.
        docker_overrides: Some(DockerOverrides {
            image: Some(docker.image.clone()),
            network: Some(docker.network.clone()),
            memory_limit_mb: docker.memory_limit_mb,
            cpu_limit: docker.cpu_limit,
            read_only_rootfs: Some(docker.read_only_rootfs),
            extra_caps_drop: vec![],
        }),
        mount_workspace: docker.mount_workspace || runtime.kind != "docker",
    }
}

/// `policy` with the loaded configuration's backend filled in when the caller named none
/// (a policy built from a default or an older configuration).
pub(crate) fn with_configured_backend(policy: &SandboxPolicy, config: &Config) -> SandboxPolicy {
    let mut policy = policy.clone();
    if policy.backend == SandboxBackendKind::None {
        policy.backend = named_backend(&config.security.sandbox, &config.runtime, false);
        if policy.backend == SandboxBackendKind::Docker {
            policy.mount_workspace =
                config.runtime.docker.mount_workspace || config.runtime.kind != "docker";
        }
    }
    policy
}

/// The policy for a command tool or scheduled job. SaaS always uses the user's container.
pub(crate) fn command_policy(
    config: &Config,
    action_dir: &Path,
    state_dir: &Path,
) -> Result<SandboxPolicy, String> {
    if crate::core::runtime::is_saas() {
        return crate::profiles::tools::sandbox_policy(action_dir, state_dir);
    }
    Ok(spawn_policy(
        action_dir,
        state_dir,
        &config.security.sandbox,
        &config.runtime,
        false,
    ))
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
