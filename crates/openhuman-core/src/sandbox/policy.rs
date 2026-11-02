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
use tinyagents_definition::SandboxMode;

/// The executor `sandbox` and `runtime` name, if any. `remote` marks a channel, cron or
/// remote session, which has always asked for a container.
fn named_backend(
    sandbox: &SandboxConfig,
    runtime: &RuntimeConfig,
    remote: bool,
) -> SandboxBackendKind {
    match sandbox.backend {
        SandboxBackend::Landlock | SandboxBackend::Firejail | SandboxBackend::Bubblewrap => {
            SandboxBackendKind::Local
        }
        SandboxBackend::Docker => SandboxBackendKind::Docker,
        SandboxBackend::OpenShell => SandboxBackendKind::OpenShell,
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

/// Route a spawn to a preferred executor; TinySecurity still plans and decides it.
///
/// `state_dir` is the core's internal `workspace_dir`; host-side scratch such as output
/// capture goes there instead of into `action_dir`. The jail's toolchain and git grants
/// are resolved by TinyBox when the spawn is planned (`sandbox::local_grants`), so the
/// policy starts without mounts.
pub fn route_for_mode(
    mode: SandboxMode,
    action_dir: &Path,
    state_dir: &Path,
    sandbox: &SandboxConfig,
    runtime: &RuntimeConfig,
    remote: bool,
) -> SandboxPolicy {
    let requested = named_backend(sandbox, runtime, remote);
    let backend = match mode {
        SandboxMode::None | SandboxMode::ReadOnly => requested,
        SandboxMode::Sandboxed if requested != SandboxBackendKind::None => requested,
        SandboxMode::Sandboxed if remote || runtime.kind == "docker" => SandboxBackendKind::Docker,
        SandboxMode::Sandboxed => SandboxBackendKind::Local,
    };
    let docker = &runtime.docker;
    // A tenant's OpenShell box has its own workspace and no egress: nothing on
    // the host is mounted into it and no container settings apply.
    let remote_box = backend == SandboxBackendKind::OpenShell;
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
        allow_network: !remote_box && (!matches!(mode, SandboxMode::Sandboxed) || !remote),
        mount_workspace: !remote_box && (docker.mount_workspace || runtime.kind != "docker"),
        env_passthrough: SANDBOX_ENV_PASSTHROUGH
            .iter()
            .map(|s| s.to_string())
            .collect(),
        // Carried whichever executor the module picks; only a container reads it.
        docker_overrides: (!remote_box).then(|| DockerOverrides {
            image: Some(docker.image.clone()),
            network: Some(docker.network.clone()),
            memory_limit_mb: docker.memory_limit_mb,
            cpu_limit: docker.cpu_limit,
            read_only_rootfs: Some(docker.read_only_rootfs),
            extra_caps_drop: vec![],
        }),
    }
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
    Ok(route_for_mode(
        SandboxMode::Sandboxed,
        action_dir,
        state_dir,
        &config.sandbox,
        &config.runtime,
        false,
    ))
}

/// Whether a command must enter the Plan2 spawn path.
pub(crate) async fn should_plan_command(
    ctx: &crate::core::runtime::DispatchCtx,
    mode: Option<SandboxMode>,
) -> Result<bool, String> {
    let untrusted = matches!(
        ctx.turn_origin_or_scoped(),
        None | Some(crate::core::runtime::turn_origin::AgentTurnOrigin::ExternalChannel { .. })
            | Some(crate::core::runtime::turn_origin::AgentTurnOrigin::Unknown)
    );
    if crate::core::runtime::is_saas() || untrusted || matches!(mode, Some(SandboxMode::Sandboxed))
    {
        return Ok(true);
    }
    let config = ctx.load_config().await?;
    Ok(backend_requires_sandbox(&config.sandbox.backend))
}

/// Whether the operator's backend selection is an explicit isolation
/// requirement, rather than leaving the choice to the agent's mode.
pub(crate) fn backend_requires_sandbox(backend: &SandboxBackend) -> bool {
    matches!(
        backend,
        SandboxBackend::Landlock
            | SandboxBackend::Firejail
            | SandboxBackend::Bubblewrap
            | SandboxBackend::Docker
            | SandboxBackend::OpenShell
    )
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
