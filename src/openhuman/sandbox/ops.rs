//! Sandbox backend operations — policy resolution, backend creation, and
//! routed execution.

use super::docker;
use super::types::{
    ElevatedOp, SandboxBackendHandle, SandboxBackendKind, SandboxExecRequest, SandboxExecResult,
    SandboxPolicy, SandboxStatus, ELEVATED_TOOLS,
};
use crate::openhuman::agent::harness::definition::SandboxMode;
use crate::openhuman::agent::platform_shell;
use crate::openhuman::config::RuntimeConfig;
use crate::openhuman::sandbox::cwd_jail::{self, Jail, NoopBackend};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

/// Safe environment variables forwarded into sandboxed execution.
pub const SANDBOX_ENV_PASSTHROUGH: &[&str] = &[
    "PATH", "HOME", "TERM", "LANG", "LC_ALL", "LC_CTYPE", "USER", "SHELL", "TMPDIR",
];

/// Resolve a `SandboxPolicy` from the agent's `SandboxMode`, the
/// session origin, and the global runtime config.
///
/// Non-main sessions (channel, cron, remote) default to `Docker` when
/// the mode is `Sandboxed` and Docker is configured. Local interactive
/// sessions default to `Local` (OS-level jail via `cwd_jail`).
pub fn resolve_sandbox_policy(
    mode: SandboxMode,
    action_dir: &Path,
    runtime_config: &RuntimeConfig,
    is_remote_session: bool,
) -> SandboxPolicy {
    let backend = match mode {
        SandboxMode::None => SandboxBackendKind::None,
        SandboxMode::ReadOnly => SandboxBackendKind::None,
        SandboxMode::Sandboxed => {
            if runtime_config.kind == "docker" || is_remote_session {
                SandboxBackendKind::Docker
            } else {
                SandboxBackendKind::Local
            }
        }
    };

    let docker_overrides = if backend == SandboxBackendKind::Docker {
        let dc = &runtime_config.docker;
        Some(super::types::DockerOverrides {
            image: Some(dc.image.clone()),
            network: Some(dc.network.clone()),
            memory_limit_mb: dc.memory_limit_mb,
            cpu_limit: dc.cpu_limit,
            read_only_rootfs: Some(dc.read_only_rootfs),
            extra_caps_drop: vec![],
        })
    } else {
        None
    };

    let allow_network = match mode {
        SandboxMode::Sandboxed => !is_remote_session,
        _ => true,
    };

    tracing::debug!(
        mode = ?mode,
        backend = ?backend,
        is_remote = is_remote_session,
        action_dir = %action_dir.display(),
        "[sandbox] resolved policy"
    );

    SandboxPolicy {
        backend,
        workspace_root: action_dir.to_path_buf(),
        read_only_mounts: vec![],
        allow_network,
        env_passthrough: SANDBOX_ENV_PASSTHROUGH
            .iter()
            .map(|s| s.to_string())
            .collect(),
        docker_overrides,
    }
}

/// Create a backend handle for the resolved policy. For Docker this
/// checks availability; for Local it checks the OS backend.
/// The status a `Local` sandbox handle reports, given the backend `pick_backend`
/// actually chose.
///
/// Extracted as a free function so the decision is testable on **any** host.
/// Asserting it through `create_sandbox_backend` cannot work: which branch runs
/// depends on whether the machine has an OS jail, so on a host with Seatbelt or
/// Landlock the noop path is never reached and a regression to "always Ready"
/// passes unnoticed. That is exactly how the original defect survived.
pub(crate) fn local_status_for_backend(backend_name: &str) -> SandboxStatus {
    if backend_name == cwd_jail::NOOP_BACKEND_NAME {
        // The noop backend enforces nothing — it spawns the command as-is. It is
        // a documented passthrough rather than a failure, so `Inactive`
        // ("backend not initialized") is the honest report, not `Error`.
        SandboxStatus::Inactive
    } else {
        SandboxStatus::Ready
    }
}

pub async fn create_sandbox_backend(policy: &SandboxPolicy) -> SandboxBackendHandle {
    match policy.backend {
        SandboxBackendKind::None => SandboxBackendHandle {
            kind: SandboxBackendKind::None,
            status: SandboxStatus::Ready,
            backend_id: None,
        },
        SandboxBackendKind::Local => {
            let os_backend = cwd_jail::default_backend();
            let backend_name = os_backend.name();
            // Derive the status from WHICH backend was chosen, not from
            // `is_available()`.
            //
            // `default_backend()` is `cwd_jail::detect::pick_backend`, which
            // already performs the availability check and substitutes
            // `NoopBackend` when no OS jail is usable. `NoopBackend::is_available()`
            // is unconditionally `true` — pinned as a contract by the #3235
            // regression test — so asking it here always answered `true` and the
            // `else` arm was unreachable. The handle therefore reported `Ready`
            // on a host with no confinement at all, which is the one direction a
            // sandbox status must never be wrong in: a caller that trusts it
            // believes commands are jailed when they run unconfined.
            //
            // The noop backend is a documented passthrough, not a failure, so
            // `Inactive` ("backend not initialized") is the honest report rather
            // than `Error`. `backend_id` still carries the name for a caller that
            // wants the specific backend.
            let status = local_status_for_backend(backend_name);
            if status == SandboxStatus::Inactive {
                tracing::warn!(
                    backend = backend_name,
                    "[sandbox:local] no OS jail available; commands run UNCONFINED and \
                     the handle reports inactive"
                );
            }
            SandboxBackendHandle {
                kind: SandboxBackendKind::Local,
                status,
                backend_id: Some(backend_name.to_string()),
            }
        }
        SandboxBackendKind::Docker => docker::docker_backend_handle().await,
    }
}

/// Execute a command through the appropriate sandbox backend.
///
/// Returns the sandboxed execution result. The caller (typically the
/// shell tool) is responsible for converting this into a `ToolResult`.
pub async fn execute_in_sandbox(
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    extra_env: HashMap<OsString, OsString>,
    timeout: Duration,
) -> anyhow::Result<SandboxExecResult> {
    // Validate the working directory up front so a missing/bad action_dir
    // surfaces an actionable, path-naming error here rather than an opaque OS
    // error 267 (ERROR_DIRECTORY) at spawn time — parity with the unsandboxed
    // `NativeRuntime::build_shell_command` guard. (#3353, Fix 2)
    //
    // The validation is host-side, so it is applied per-backend: for None/Local
    // `working_dir` *is* a host path; for Docker `working_dir` is the
    // container-side mount target (e.g. `/workspace`) which must NOT be
    // stat'd/created on the host — there we validate the host-side mount source
    // (`policy.workspace_root`) instead.
    match policy.backend {
        SandboxBackendKind::None => {
            crate::openhuman::config::ensure_usable_cwd(working_dir)?;
            execute_unsandboxed(command, working_dir, &extra_env, timeout).await
        }
        SandboxBackendKind::Local => {
            crate::openhuman::config::ensure_usable_cwd(working_dir)?;
            execute_local_jail(policy, command, working_dir, &extra_env, timeout).await
        }
        SandboxBackendKind::Docker => {
            crate::openhuman::config::ensure_usable_cwd(&policy.workspace_root)?;
            let request = SandboxExecRequest {
                command: command.to_string(),
                working_dir: working_dir.to_path_buf(),
                env: extra_env,
                timeout,
            };
            docker::docker_exec(policy, &request).await
        }
    }
}

/// Passthrough execution with no sandbox (for `SandboxBackendKind::None`).
async fn execute_unsandboxed(
    command: &str,
    working_dir: &Path,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
) -> anyhow::Result<SandboxExecResult> {
    // Shell selection routed through `platform_shell` so this path picks
    // `cmd.exe /C` on Windows instead of the non-existent `sh` binary
    // (#4705 — Windows Shell tool spawn-failed at ~30ms).
    let mut cmd = platform_shell::build_tokio_command(command);
    cmd.current_dir(working_dir);
    cmd.env_clear();
    for var in SANDBOX_ENV_PASSTHROUGH {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }
    for (k, v) in extra_env {
        cmd.env(k, v);
    }

    let result = tokio::time::timeout(timeout, cmd.output()).await;
    match result {
        Ok(Ok(output)) => Ok(SandboxExecResult {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            timed_out: false,
        }),
        Ok(Err(e)) => anyhow::bail!("Failed to execute command: {e}"),
        Err(_) => Ok(SandboxExecResult {
            exit_code: -1,
            stdout: String::new(),
            stderr: format!("Command timed out after {}s", timeout.as_secs()),
            timed_out: true,
        }),
    }
}

/// Execute via the OS-level `cwd_jail` backend (Landlock/Seatbelt/AppContainer).
///
/// Output capture: some OS backends (macOS Seatbelt) rebuild the command
/// internally and don't forward piped stdio settings. We capture output
/// by wrapping the command to redirect stdout/stderr to temp files inside
/// the jail root, then reading them back after exit.
async fn execute_local_jail(
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
) -> anyhow::Result<SandboxExecResult> {
    let mut jail = Jail::new(&policy.workspace_root, "sandbox.agent");
    if !policy.allow_network {
        jail = jail.deny_net();
    }
    for ro in &policy.read_only_mounts {
        jail = jail.add_read_only(ro);
    }

    let stdout_file = policy.workspace_root.join(".sandbox_stdout");
    let stderr_file = policy.workspace_root.join(".sandbox_stderr");
    // Platform-aware output-capture wrap: `{ … ; } > … 2> …` on sh/bash,
    // trailing `> … 2> …` on cmd.exe (no brace grouping). Shell binary is
    // picked by `platform_shell` so this path is Windows-safe (#4705).
    let wrapped = platform_shell::wrap_with_output_redirection(command, &stdout_file, &stderr_file);
    let mut cmd = platform_shell::build_std_command(&wrapped);
    cmd.current_dir(working_dir);
    cmd.env_clear();
    for var in SANDBOX_ENV_PASSTHROUGH {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }
    for (k, v) in extra_env {
        cmd.env(k, v);
    }

    let os_backend = cwd_jail::default_backend();
    let spawn_result = if os_backend.is_available() {
        cwd_jail::spawn(&jail, cmd)
    } else {
        tracing::debug!("[sandbox:local] OS backend unavailable, using noop");
        cwd_jail::spawn_with(&NoopBackend, &jail, cmd)
    };

    let stdout_path = stdout_file.clone();
    let stderr_path = stderr_file.clone();

    match spawn_result {
        Ok(child) => {
            let wait_result = tokio::task::spawn_blocking(move || {
                let start = std::time::Instant::now();
                let mut child = child;
                loop {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            return Ok((status.code().unwrap_or(-1), false));
                        }
                        Ok(None) => {
                            if start.elapsed() > timeout {
                                let _ = child.kill();
                                return Ok((-1, true));
                            }
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        Err(e) => return Err(e),
                    }
                }
            })
            .await??;

            let stdout = std::fs::read_to_string(&stdout_path).unwrap_or_default();
            let stderr_content = std::fs::read_to_string(&stderr_path).unwrap_or_default();
            let _ = std::fs::remove_file(&stdout_path);
            let _ = std::fs::remove_file(&stderr_path);

            Ok(SandboxExecResult {
                exit_code: wait_result.0,
                stdout,
                stderr: if wait_result.1 {
                    format!("Command timed out after {}s", timeout.as_secs())
                } else {
                    stderr_content
                },
                timed_out: wait_result.1,
            })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&stdout_path);
            let _ = std::fs::remove_file(&stderr_path);
            anyhow::bail!("Failed to spawn jailed process: {e}")
        }
    }
}

/// Check whether a tool operation is an elevated op that must run on the
/// host even when the session is sandboxed.
pub fn is_elevated_op(tool_name: &str) -> bool {
    ELEVATED_TOOLS.contains(&tool_name)
}

/// Build an `ElevatedOp` for audit logging when a tool bypasses the sandbox.
pub fn build_elevated_op(tool_name: &str, command: &str, reason: &str) -> ElevatedOp {
    tracing::info!(
        tool = tool_name,
        reason = reason,
        "[sandbox] elevated host operation"
    );
    ElevatedOp {
        tool_name: tool_name.to_string(),
        reason: reason.to_string(),
        command: command.to_string(),
    }
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
