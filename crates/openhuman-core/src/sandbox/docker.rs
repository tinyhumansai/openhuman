//! Docker-backed sandbox execution backend.
//!
//! Policy mapping for the Docker sandbox; TinyBox owns one-shot container
//! creation, execution, and bounded output collection. Commands run inside
//! ephemeral containers with:
//! - Controlled workspace mounts (host `action_dir` → `/workspace`)
//! - Network isolation (default: `none`)
//! - Resource limits (memory, CPU)
//! - Capability dropping (`--cap-drop ALL`)
//! - Read-only rootfs (configurable)
//! - Environment passthrough (explicit allowlist only)
//! - Automatic container cleanup on completion
//!
//! The host core process is never inside the container — only the
//! spawned command runs sandboxed.

use super::types::{
    SandboxBackendHandle, SandboxBackendKind, SandboxExecRequest, SandboxExecResult, SandboxPolicy,
    SandboxStatus,
};
use crate::config::Config;
#[cfg(feature = "modules")]
use crate::modules::tinybox::{self, contract};

/// Label applied to all sandbox containers for orphan cleanup.
const CONTAINER_LABEL: &str = "openhuman.sandbox=true";

#[cfg(feature = "modules")]
fn status_request() -> contract::DockerStatusRequest {
    contract::DockerStatusRequest {
        timeout_ms: contract::DEFAULT_DOCKER_CONTROL_TIMEOUT_MS,
    }
}

#[cfg(feature = "modules")]
fn cleanup_request() -> contract::CleanupDockerRequest {
    contract::CleanupDockerRequest {
        label: CONTAINER_LABEL.to_owned(),
        timeout_ms: contract::DEFAULT_DOCKER_CONTROL_TIMEOUT_MS,
    }
}

/// Check whether Docker is available and responsive.
#[cfg(feature = "modules")]
pub async fn is_docker_available(config: &Config) -> Result<bool, String> {
    let status = tinybox::docker_status(config, status_request()).await?;
    Ok(status.available)
}

#[cfg(not(feature = "modules"))]
pub async fn is_docker_available(_config: &Config) -> Result<bool, String> {
    Err("TinyBox module support is disabled; Docker status is unavailable".to_owned())
}

/// Execute a command inside an ephemeral Docker container.
///
/// Policy stays here (which image, network, limits and mounts the
/// `SandboxPolicy` asks for); TinyBox owns the one-shot container operation.
/// There is no local Docker execution fallback when the module is unavailable.
#[cfg(feature = "modules")]
pub async fn docker_exec(
    config: &crate::config::Config,
    policy: &SandboxPolicy,
    request: &SandboxExecRequest,
) -> anyhow::Result<SandboxExecResult> {
    let run = build_docker_request(policy, request)?;

    tracing::debug!("[sandbox:docker] launching container");

    // A plan with no workspace grant (`mount_workspace = false`) runs the container with
    // nothing of the host mounted, through the module's workspace-less member.
    let output = if policy.mount_workspace {
        crate::modules::tinybox::run_docker(config, run).await
    } else {
        crate::modules::tinybox::run_docker_without_workspace(config, run).await
    };
    match output {
        Ok(out) if out.timed_out => {
            tracing::warn!(
                timeout_secs = request.timeout.as_secs(),
                "[sandbox:docker] container timed out, killing"
            );
            Ok(SandboxExecResult {
                exit_code: out.exit_code,
                stdout: out.stdout,
                stderr: out.stderr,
                timed_out: true,
            })
        }
        Ok(out) => {
            tracing::debug!(
                exit_code = out.exit_code,
                stdout_len = out.stdout.len(),
                stderr_len = out.stderr.len(),
                "[sandbox:docker] container exited"
            );
            Ok(SandboxExecResult {
                exit_code: out.exit_code,
                stdout: out.stdout,
                stderr: out.stderr,
                timed_out: false,
            })
        }
        Err(error) => anyhow::bail!("Docker execution failed: {error}"),
    }
}

#[cfg(not(feature = "modules"))]
pub async fn docker_exec(
    _config: &crate::config::Config,
    _policy: &SandboxPolicy,
    _request: &SandboxExecRequest,
) -> anyhow::Result<SandboxExecResult> {
    anyhow::bail!("TinyBox module support is disabled; Docker sandbox execution is unavailable")
}

#[cfg(feature = "modules")]
fn build_docker_request(
    policy: &SandboxPolicy,
    request: &SandboxExecRequest,
) -> anyhow::Result<crate::modules::tinybox::contract::DockerRunRequest> {
    use crate::modules::tinybox::contract::{DockerRunRequest, MAX_DOCKER_RUN_TIMEOUT_MS};
    use std::collections::BTreeMap;

    // The workspace-less member requires the workspace to be empty.
    let workspace = if policy.mount_workspace {
        path_string(&policy.workspace_root, "workspace")?
    } else {
        String::new()
    };
    let read_only_mounts = policy
        .read_only_mounts
        .iter()
        .map(|path| path_string(path, "read-only mount"))
        .collect::<anyhow::Result<Vec<_>>>()?;

    // Keep the existing allowlist as the only inherited environment surface.
    // Per-request values override inherited values with the same name.
    let mut env = BTreeMap::new();
    for name in &policy.env_passthrough {
        if let Ok(value) = std::env::var(name) {
            env.insert(name.clone(), value);
        }
    }
    for (name, value) in &request.env {
        let name = name
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Docker environment name is not UTF-8"))?;
        let value = value
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Docker environment value is not UTF-8"))?;
        env.insert(name.to_owned(), value.to_owned());
    }

    let timeout_ms = request
        .timeout
        .as_nanos()
        .saturating_add(999_999)
        .checked_div(1_000_000)
        .unwrap_or(0)
        .max(1);
    let timeout_ms = u64::try_from(timeout_ms)
        .map_err(|_| anyhow::anyhow!("Docker timeout exceeds the supported limit"))?;
    if timeout_ms > MAX_DOCKER_RUN_TIMEOUT_MS {
        anyhow::bail!("Docker timeout exceeds the supported limit");
    }

    let overrides = policy.docker_overrides.as_ref();
    Ok(DockerRunRequest {
        workspace,
        image: overrides.and_then(|config| config.image.clone()),
        command: request.command.clone(),
        login_shell: false,
        network: overrides.and_then(|config| config.network.clone()),
        memory_limit_mb: overrides.and_then(|config| config.memory_limit_mb),
        cpus: overrides.and_then(|config| config.cpu_limit),
        read_only_rootfs: overrides.and_then(|config| config.read_only_rootfs),
        read_only_mounts,
        extra_cap_drops: overrides
            .map(|config| config.extra_caps_drop.clone())
            .unwrap_or_default(),
        env,
        label: Some(CONTAINER_LABEL.to_owned()),
        timeout_ms,
    })
}

#[cfg(feature = "modules")]
fn path_string(path: &std::path::Path, kind: &str) -> anyhow::Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("Docker {kind} path is not UTF-8"))
}

/// Clean up orphaned sandbox containers owned by TinyBox and carrying the
/// OpenHuman label.
#[cfg(feature = "modules")]
pub async fn cleanup_orphaned_containers(config: &Config) -> anyhow::Result<u32> {
    let output = tinybox::cleanup_docker(config, cleanup_request())
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let count = output.containers_stopped;
    tracing::debug!(count = count, "[sandbox:docker] orphan cleanup finished");
    Ok(count)
}

#[cfg(not(feature = "modules"))]
pub async fn cleanup_orphaned_containers(_config: &Config) -> anyhow::Result<u32> {
    anyhow::bail!("TinyBox module support is disabled; Docker cleanup is unavailable")
}

/// Create a handle representing the Docker backend state.
pub async fn docker_backend_handle(config: &Config) -> Result<SandboxBackendHandle, String> {
    let available = is_docker_available(config).await?;
    Ok(docker_handle_for_status(available))
}

fn docker_handle_for_status(available: bool) -> SandboxBackendHandle {
    SandboxBackendHandle {
        kind: SandboxBackendKind::Docker,
        status: if available {
            SandboxStatus::Ready
        } else {
            SandboxStatus::Error
        },
        backend_id: None,
        isolation: None,
    }
}

/// The parts of a sandbox policy the module's Docker run policy judges.
#[cfg(feature = "modules")]
pub(crate) fn policy_request(policy: &SandboxPolicy) -> contract::DockerPolicyRequest {
    contract::DockerPolicyRequest {
        network: policy
            .docker_overrides
            .as_ref()
            .and_then(|overrides| overrides.network.clone()),
        workspace: policy
            .mount_workspace
            .then(|| policy.workspace_root.to_string_lossy().into_owned()),
        read_only_mounts: policy
            .read_only_mounts
            .iter()
            .map(|mount| mount.to_string_lossy().into_owned())
            .collect(),
    }
}

/// Ask TinyBox whether a sandbox policy's Docker configuration has dangerous
/// settings (host network, privileged mounts, ...).
///
/// The verdict is the module's: the same check `RunDocker` enforces before it
/// creates a container. `Ok` carries the findings (empty means safe); an `Err`
/// means no verdict could be obtained, which callers must not read as safe.
#[cfg(feature = "modules")]
pub async fn validate_docker_policy(
    config: &Config,
    policy: &SandboxPolicy,
) -> Result<Vec<String>, String> {
    let report = tinybox::validate_docker_policy(config, policy_request(policy)).await?;
    tracing::debug!(
        issues = report.issues.len(),
        "[sandbox:docker] TinyBox validated the policy"
    );
    Ok(report.issues)
}

#[cfg(not(feature = "modules"))]
pub async fn validate_docker_policy(
    _config: &Config,
    _policy: &SandboxPolicy,
) -> Result<Vec<String>, String> {
    Err("TinyBox module support is disabled; policy validation is unavailable".to_owned())
}

#[cfg(test)]
#[path = "docker_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "docker_exec_tests.rs"]
mod exec_tests;
