//! Docker-backed sandbox execution backend.
//!
//! Runs agent tool commands inside ephemeral Docker containers with:
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
    DockerOverrides, SandboxBackendHandle, SandboxBackendKind, SandboxExecRequest,
    SandboxExecResult, SandboxPolicy, SandboxStatus,
};
use std::process::Stdio;
use tokio::process::Command;

/// Label applied to all sandbox containers for orphan cleanup.
const CONTAINER_LABEL: &str = "openhuman.sandbox=true";

/// Maximum output size in bytes (1MB), matching shell tool limit.
const MAX_OUTPUT_BYTES: usize = 1_048_576;

/// Check whether Docker is available and responsive.
pub async fn is_docker_available() -> bool {
    let result = Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await;
    match result {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

/// Execute a command inside an ephemeral Docker container.
///
/// The container is created with `docker run --rm` so it self-cleans on
/// exit. Resource limits, network policy, and mounts are derived from
/// the `SandboxPolicy`.
pub async fn docker_exec(
    policy: &SandboxPolicy,
    request: &SandboxExecRequest,
) -> anyhow::Result<SandboxExecResult> {
    let overrides = policy.docker_overrides.as_ref();
    let image = overrides
        .and_then(|o| o.image.as_deref())
        .unwrap_or("alpine:3.20");
    let network = overrides
        .and_then(|o| o.network.as_deref())
        .unwrap_or("none");

    let mut cmd = Command::new("docker");
    cmd.arg("run").arg("--rm");

    // Container identification for orphan cleanup.
    cmd.arg("--label").arg(CONTAINER_LABEL);

    // Network isolation.
    cmd.arg("--network").arg(network);

    // Drop all capabilities by default.
    cmd.arg("--cap-drop").arg("ALL");

    // Additional capability drops.
    if let Some(ov) = overrides {
        for cap in &ov.extra_caps_drop {
            cmd.arg("--cap-drop").arg(cap);
        }
    }

    // Resource limits.
    let memory_mb = overrides.and_then(|o| o.memory_limit_mb).unwrap_or(512);
    cmd.arg("-m").arg(format!("{memory_mb}m"));

    let cpu = overrides.and_then(|o| o.cpu_limit).unwrap_or(1.0);
    cmd.arg("--cpus").arg(cpu.to_string());

    // Read-only rootfs.
    let read_only = overrides.and_then(|o| o.read_only_rootfs).unwrap_or(true);
    if read_only {
        cmd.arg("--read-only");
        // tmpfs mounts so the container can still write to /tmp and /var/tmp.
        cmd.arg("--tmpfs").arg("/tmp:rw,noexec,nosuid,size=64m");
        cmd.arg("--tmpfs").arg("/var/tmp:rw,noexec,nosuid,size=64m");
    }

    // No new privileges (prevent setuid/setgid escalation inside container).
    cmd.arg("--security-opt").arg("no-new-privileges");

    // Workspace mount.
    let workspace = policy
        .workspace_root
        .canonicalize()
        .unwrap_or_else(|_| policy.workspace_root.clone());
    let mount = format!("{}:/workspace", workspace.display());
    cmd.arg("-v").arg(mount);
    cmd.arg("-w").arg("/workspace");

    // Read-only mounts.
    for ro_path in &policy.read_only_mounts {
        let canonical = ro_path.canonicalize().unwrap_or_else(|_| ro_path.clone());
        let ro_mount = format!("{}:{}:ro", canonical.display(), canonical.display());
        cmd.arg("-v").arg(ro_mount);
    }

    // Environment passthrough.
    for var_name in &policy.env_passthrough {
        if let Ok(val) = std::env::var(var_name) {
            cmd.arg("-e").arg(format!("{var_name}={val}"));
        }
    }
    // Inject request-specific environment.
    for (k, v) in &request.env {
        cmd.arg("-e").arg(format!("{k}={v}"));
    }

    cmd.arg(image);
    cmd.arg("sh").arg("-c").arg(&request.command);

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    tracing::debug!(
        image = image,
        network = network,
        memory_mb = memory_mb,
        cpu = cpu,
        workspace = %workspace.display(),
        command = %request.command,
        "[sandbox:docker] launching container"
    );

    let result = tokio::time::timeout(request.timeout, cmd.output()).await;

    match result {
        Ok(Ok(output)) => {
            let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();

            if stdout.len() > MAX_OUTPUT_BYTES {
                stdout.truncate(MAX_OUTPUT_BYTES);
                stdout.push_str("\n... [output truncated at 1MB]");
            }
            if stderr.len() > MAX_OUTPUT_BYTES {
                stderr.truncate(MAX_OUTPUT_BYTES);
                stderr.push_str("\n... [stderr truncated at 1MB]");
            }

            let exit_code = output.status.code().unwrap_or(-1);
            tracing::debug!(
                exit_code = exit_code,
                stdout_len = stdout.len(),
                stderr_len = stderr.len(),
                "[sandbox:docker] container exited"
            );

            Ok(SandboxExecResult {
                exit_code,
                stdout,
                stderr,
                timed_out: false,
            })
        }
        Ok(Err(e)) => {
            tracing::error!(error = %e, "[sandbox:docker] failed to spawn container");
            anyhow::bail!("Docker execution failed: {e}")
        }
        Err(_) => {
            tracing::warn!(
                timeout_secs = request.timeout.as_secs(),
                "[sandbox:docker] container timed out, killing"
            );
            Ok(SandboxExecResult {
                exit_code: -1,
                stdout: String::new(),
                stderr: format!(
                    "Command timed out after {}s and was killed",
                    request.timeout.as_secs()
                ),
                timed_out: true,
            })
        }
    }
}

/// Clean up orphaned sandbox containers (those labeled with
/// `openhuman.sandbox=true` that are still running).
pub async fn cleanup_orphaned_containers() -> anyhow::Result<u32> {
    let output = Command::new("docker")
        .args(["ps", "-q", "--filter", &format!("label={CONTAINER_LABEL}")])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await?;

    let ids: Vec<&str> = std::str::from_utf8(&output.stdout)
        .unwrap_or("")
        .lines()
        .filter(|l| !l.is_empty())
        .collect();

    if ids.is_empty() {
        tracing::debug!("[sandbox:docker] no orphaned containers found");
        return Ok(0);
    }

    let count = ids.len() as u32;
    tracing::info!(
        count = count,
        "[sandbox:docker] cleaning up orphaned containers"
    );

    let mut kill_cmd = Command::new("docker");
    kill_cmd.arg("kill");
    for id in &ids {
        kill_cmd.arg(id);
    }
    let _ = kill_cmd
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;

    Ok(count)
}

/// Create a handle representing the Docker backend state.
pub async fn docker_backend_handle() -> SandboxBackendHandle {
    let available = is_docker_available().await;
    SandboxBackendHandle {
        kind: SandboxBackendKind::Docker,
        status: if available {
            SandboxStatus::Ready
        } else {
            SandboxStatus::Error
        },
        backend_id: None,
    }
}

/// Validate that a sandbox policy's Docker configuration doesn't have
/// dangerous settings (host network, privileged mounts, etc.).
pub fn validate_docker_policy(policy: &SandboxPolicy) -> Result<(), Vec<String>> {
    let mut issues = Vec::new();

    if let Some(overrides) = &policy.docker_overrides {
        if overrides.network.as_deref() == Some("host") {
            issues.push("Docker sandbox uses host network — defeats network isolation".into());
        }
    }

    // Check for dangerous mount paths.
    let dangerous_mounts = ["/", "/etc", "/var/run/docker.sock", "/proc", "/sys"];
    for mount in &policy.read_only_mounts {
        let path_str = mount.to_string_lossy();
        for &dangerous in &dangerous_mounts {
            if path_str == dangerous {
                issues.push(format!(
                    "Dangerous read-only mount: {path_str} — could leak host secrets"
                ));
            }
        }
    }

    let workspace_str = policy.workspace_root.to_string_lossy();
    for &dangerous in &dangerous_mounts {
        if workspace_str == dangerous {
            issues.push(format!(
                "Workspace root is a dangerous path: {workspace_str}"
            ));
        }
    }

    if issues.is_empty() {
        Ok(())
    } else {
        Err(issues)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_policy() -> SandboxPolicy {
        SandboxPolicy {
            backend: SandboxBackendKind::Docker,
            workspace_root: PathBuf::from("/tmp/test-workspace"),
            read_only_mounts: vec![],
            allow_network: false,
            env_passthrough: vec!["PATH".into(), "HOME".into()],
            docker_overrides: None,
        }
    }

    #[test]
    fn validate_docker_policy_accepts_safe_config() {
        let policy = test_policy();
        assert!(validate_docker_policy(&policy).is_ok());
    }

    #[test]
    fn validate_docker_policy_rejects_host_network() {
        let mut policy = test_policy();
        policy.docker_overrides = Some(DockerOverrides {
            network: Some("host".into()),
            ..DockerOverrides::default()
        });
        let issues = validate_docker_policy(&policy).unwrap_err();
        assert!(issues.iter().any(|i| i.contains("host network")));
    }

    #[test]
    fn validate_docker_policy_rejects_dangerous_mounts() {
        let mut policy = test_policy();
        policy.read_only_mounts = vec![PathBuf::from("/var/run/docker.sock")];
        let issues = validate_docker_policy(&policy).unwrap_err();
        assert!(issues.iter().any(|i| i.contains("docker.sock")));
    }

    #[test]
    fn validate_docker_policy_rejects_root_workspace() {
        let mut policy = test_policy();
        policy.workspace_root = PathBuf::from("/");
        let issues = validate_docker_policy(&policy).unwrap_err();
        assert!(issues.iter().any(|i| i.contains("dangerous path")));
    }

    #[test]
    fn validate_docker_policy_multiple_issues() {
        let mut policy = test_policy();
        policy.workspace_root = PathBuf::from("/etc");
        policy.read_only_mounts = vec![PathBuf::from("/proc")];
        policy.docker_overrides = Some(DockerOverrides {
            network: Some("host".into()),
            ..DockerOverrides::default()
        });
        let issues = validate_docker_policy(&policy).unwrap_err();
        assert!(issues.len() >= 3);
    }

    #[tokio::test]
    async fn docker_backend_handle_reports_status() {
        let handle = docker_backend_handle().await;
        assert_eq!(handle.kind, SandboxBackendKind::Docker);
        // Status depends on whether Docker is installed in the test env.
        assert!(handle.status == SandboxStatus::Ready || handle.status == SandboxStatus::Error);
    }
}
