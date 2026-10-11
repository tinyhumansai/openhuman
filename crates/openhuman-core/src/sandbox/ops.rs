//! Sandbox backend operations — policy resolution, backend creation, and
//! routed execution.

use super::docker;
use super::grants::{resolve_local_jail_grants, JailGrants};
use super::types::{
    ElevatedOp, SandboxBackendHandle, SandboxBackendKind, SandboxExecRequest, SandboxExecResult,
    SandboxPolicy, SandboxStatus, ELEVATED_TOOLS,
};
use crate::agent::harness::definition::SandboxMode;
use crate::agent::platform_shell;
use crate::config::{RuntimeConfig, SandboxBackend, SandboxConfig};
use crate::sandbox::cwd_jail::{self, Jail, NoopBackend};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Safe environment variables forwarded into sandboxed execution.
///
/// This list is the *entire* environment a sandboxed child gets: both
/// [`execute_unsandboxed`] and [`execute_local_jail`] call `env_clear()` and
/// re-forward only what is named here. Windows process-bootstrap variables are
/// added only in the host spawn paths below; keeping them out of this policy
/// prevents Windows paths from being passed into Linux Docker containers.
///
/// They were missing when this defect was found, and it survived the first
/// round of fixes because the tool launchers now share the shell and flows code-runner paths rather than
/// carrying separate copies of the allow-list and
/// had already been patched: the built-in `orchestrator` runs with
/// `sandbox_mode = "sandboxed"`, and all four tools divert to
/// [`crate::sandbox`] *before* reaching those lists, so the host spawn paths
/// below are the ones that must add the Windows-only bootstrap set.
pub const SANDBOX_ENV_PASSTHROUGH: &[&str] = &[
    "PATH", "HOME", "TERM", "LANG", "LC_ALL", "LC_CTYPE", "USER", "SHELL", "TMPDIR",
];

/// Host switch that turns the agent sandbox off for the whole process.
///
/// For hosts that already isolate the core (a container, a CI or benchmark
/// task image, a VM): the OS jail confines commands to the action dir, which
/// breaks work such as installing packages or editing `/etc` that the outer
/// isolation already permits. `off`, `none`, `0`, `false` or `disabled`
/// (case-insensitive) disable it; anything else, or unset, leaves it on.
pub const SANDBOX_OFF_ENV: &str = "OPENHUMAN_SANDBOX";

/// Whether `value` (the `OPENHUMAN_SANDBOX` setting) disables the sandbox.
pub fn sandbox_off_value(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("off" | "none" | "0" | "false" | "disabled")
    )
}

/// Whether a command tool must use the sandbox execution path. An explicit
/// backend in the active config is an operator requirement even when the
/// agent definition uses the usual `SandboxMode::None` default.
pub async fn command_requires_sandbox() -> Result<bool, String> {
    if crate::core::runtime::is_saas() {
        // Every SaaS command must enter the per-user container path, including
        // language tools whose agent definition does not request a sandbox.
        return Ok(true);
    }
    if sandbox_disabled_by_host() {
        return Ok(false);
    }
    if matches!(
        crate::agent::harness::current_sandbox_mode(),
        Some(SandboxMode::Sandboxed)
    ) {
        return Ok(true);
    }
    let config = crate::config::ops::load_config_with_timeout().await?;
    if config.sandbox.enabled == Some(false) {
        return Ok(false);
    }
    Ok(matches!(
        config.sandbox.backend,
        SandboxBackend::Landlock
            | SandboxBackend::Firejail
            | SandboxBackend::Bubblewrap
            | SandboxBackend::Docker
    ))
}

/// Resolve the policy for a command tool before building its command line.
/// SaaS always uses the user's container; other hosts apply the operator's
/// backend selection while retaining an agent-level sandbox requirement.
pub(crate) async fn resolve_command_policy(
    action_dir: &Path,
    state_dir: &Path,
) -> Result<SandboxPolicy, String> {
    if crate::core::runtime::is_saas() {
        return crate::profiles::tools::sandbox_policy(action_dir, state_dir);
    }
    let config = crate::config::ops::load_config_with_timeout()
        .await
        .map_err(|error| format!("Cannot read sandbox configuration: {error}"))?;
    let mut policy = resolve_sandbox_policy(
        SandboxMode::Sandboxed,
        action_dir,
        state_dir,
        &config.runtime,
        false,
    );
    if config.sandbox.enabled != Some(false) && !sandbox_disabled_by_host() {
        apply_requested_backend(&mut policy, &config.sandbox, &config.runtime)
            .map_err(|error| error.to_string())?;
    }
    Ok(policy)
}

fn sandbox_disabled_by_host() -> bool {
    sandbox_off_value(std::env::var(SANDBOX_OFF_ENV).ok().as_deref())
}

/// Resolve a `SandboxPolicy` from the agent's `SandboxMode`, the
/// session origin, and the global runtime config.
///
/// Non-main sessions (channel, cron, remote) default to `Docker` when
/// the mode is `Sandboxed` and Docker is configured. Local interactive
/// sessions default to `Local` (OS-level jail via `cwd_jail`).
///
/// `state_dir` is the core's internal `workspace_dir`; host-side scratch such
/// as output capture goes there instead of into `action_dir`.
pub fn resolve_sandbox_policy(
    mode: SandboxMode,
    action_dir: &Path,
    state_dir: &Path,
    runtime_config: &RuntimeConfig,
    is_remote_session: bool,
) -> SandboxPolicy {
    let backend = match mode {
        SandboxMode::None => SandboxBackendKind::None,
        SandboxMode::ReadOnly => SandboxBackendKind::None,
        SandboxMode::Sandboxed if sandbox_disabled_by_host() => {
            tracing::debug!(
                env = SANDBOX_OFF_ENV,
                "[sandbox] host disabled the sandbox; running sandboxed mode unconfined"
            );
            SandboxBackendKind::None
        }
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

    // The jail denies whatever it is not told about, so a local policy carries
    // the toolchain/git grants everyday commands need (see `sandbox::grants`).
    let grants = if backend == SandboxBackendKind::Local {
        host_local_jail_grants(runtime_config)
    } else {
        JailGrants::default()
    };

    tracing::debug!(
        mode = ?mode,
        backend = ?backend,
        is_remote = is_remote_session,
        action_dir = %action_dir.display(),
        read_only_grants = grants.read_only.len(),
        read_write_grants = grants.read_write.len(),
        "[sandbox] resolved policy"
    );

    SandboxPolicy {
        backend,
        workspace_root: action_dir.to_path_buf(),
        state_dir: state_dir.to_path_buf(),
        read_only_mounts: grants.read_only,
        read_write_mounts: grants.read_write,
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
    if backend_name == cwd_jail::NOOP_BACKEND_NAME
        || backend_name == cwd_jail::detect::UNSUPPORTED_BACKEND_NAME
    {
        // The noop backend enforces nothing — it spawns the command as-is. The
        // `unsupported` backend is what `pick_backend` answers when no OS jail
        // is usable (no Landlock in the kernel, Windows, ...); it refuses to
        // spawn, and `execute_local_jail` then falls back to the noop backend,
        // so commands still run unconfined. Both are a documented passthrough
        // rather than a failure, so `Inactive` ("backend not initialized") is
        // the honest report, not `Error`.
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
    // The saved backend is an operator's explicit isolation requirement. It
    // must be checked at the spawn boundary, including callers that resolved
    // their policy from an older or default RuntimeConfig.
    let mut effective_policy = policy.clone();
    if !crate::core::runtime::is_saas() && sandbox_disabled_by_host() {
        effective_policy.backend = SandboxBackendKind::None;
    }
    let explicitly_requested = if crate::core::runtime::is_saas() || sandbox_disabled_by_host() {
        false
    } else {
        let config = crate::config::ops::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("Cannot read sandbox configuration: {e}"))?;
        if config.sandbox.enabled == Some(false) {
            false
        } else {
            apply_requested_backend(&mut effective_policy, &config.sandbox, &config.runtime)?
        }
    };
    let policy = &effective_policy;
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
            crate::config::ensure_usable_cwd(working_dir)?;
            execute_unsandboxed(command, working_dir, &extra_env, timeout).await
        }
        SandboxBackendKind::Local => {
            crate::config::ensure_usable_cwd(working_dir)?;
            execute_local_jail(
                policy,
                command,
                working_dir,
                &extra_env,
                timeout,
                explicitly_requested,
            )
            .await
        }
        SandboxBackendKind::Docker => {
            crate::config::ensure_usable_cwd(&policy.workspace_root)?;
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

/// Apply an operator-selected backend. Auto retains the caller's existing
/// policy. None opts out only when no agent-level sandbox is required.
pub(crate) fn apply_requested_backend(
    policy: &mut SandboxPolicy,
    sandbox: &SandboxConfig,
    runtime: &RuntimeConfig,
) -> anyhow::Result<bool> {
    match &sandbox.backend {
        SandboxBackend::Auto => Ok(false),
        SandboxBackend::None => Ok(false),
        SandboxBackend::Docker => {
            if policy.backend == SandboxBackendKind::Local {
                policy.read_only_mounts.clear();
                policy.read_write_mounts.clear();
            }
            policy.backend = SandboxBackendKind::Docker;
            let dc = &runtime.docker;
            policy.docker_overrides = Some(super::types::DockerOverrides {
                image: Some(dc.image.clone()),
                network: Some(dc.network.clone()),
                memory_limit_mb: dc.memory_limit_mb,
                cpu_limit: dc.cpu_limit,
                read_only_rootfs: Some(dc.read_only_rootfs),
                extra_caps_drop: vec![],
            });
            Ok(true)
        }
        SandboxBackend::Landlock => {
            let backend = cwd_jail::default_backend();
            if backend.name() != "landlock" || !backend.is_available() {
                anyhow::bail!("Landlock sandbox was explicitly requested but is unavailable");
            }
            if policy.backend != SandboxBackendKind::Local {
                let grants = host_local_jail_grants(runtime);
                policy.read_only_mounts = grants.read_only;
                policy.read_write_mounts = grants.read_write;
            }
            policy.backend = SandboxBackendKind::Local;
            policy.docker_overrides = None;
            Ok(true)
        }
        SandboxBackend::Firejail => {
            anyhow::bail!("Firejail sandbox was explicitly requested but this build has no Firejail execution backend")
        }
        SandboxBackend::Bubblewrap => {
            anyhow::bail!("Bubblewrap sandbox was explicitly requested but this build has no Bubblewrap execution backend")
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
            if val.is_empty() {
                anyhow::bail!("sandbox passthrough environment variable {var} is empty");
            }
            cmd.env(var, val);
        }
    }
    platform_shell::forward_windows_bootstrap_env(&mut cmd)?;
    for (k, v) in extra_env {
        cmd.env(k, v);
    }

    let result = crate::tools::timeout::output_or_kill(&mut cmd, timeout).await;
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

/// Where the local jail captures command output:
/// `<state_dir>/artifacts/sandbox-capture`. One directory per call lives under
/// it and is removed once the output has been read.
pub fn sandbox_capture_root(state_dir: &Path) -> PathBuf {
    state_dir.join("artifacts").join("sandbox-capture")
}

/// Where the local jail's per-call writable scratch lives (`TMPDIR` points
/// here): `<state_dir>/artifacts/sandbox-scratch`.
pub fn sandbox_scratch_root(state_dir: &Path) -> PathBuf {
    state_dir.join("artifacts").join("sandbox-scratch")
}

/// A fresh per-call directory, `<root>/<uuid>`, removed with its contents on
/// drop so every exit path (spawn failure, wait error, timeout) cleans up.
struct CallDir {
    path: PathBuf,
    kind: &'static str,
}

impl CallDir {
    fn create(root: PathBuf, kind: &'static str) -> anyhow::Result<Self> {
        let path = root.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&path).map_err(|e| {
            anyhow::anyhow!(
                "Failed to create sandbox {kind} dir {}: {e}",
                path.display()
            )
        })?;
        tracing::debug!(dir = %path.display(), kind, "[sandbox:local] created per-call dir");
        Ok(Self { path, kind })
    }

    fn stdout(&self) -> PathBuf {
        self.path.join("stdout")
    }

    fn stderr(&self) -> PathBuf {
        self.path.join("stderr")
    }
}

impl Drop for CallDir {
    fn drop(&mut self) {
        match std::fs::remove_dir_all(&self.path) {
            Ok(()) => tracing::debug!(
                dir = %self.path.display(),
                kind = self.kind,
                "[sandbox:local] removed per-call dir"
            ),
            Err(e) => tracing::warn!(
                dir = %self.path.display(),
                kind = self.kind,
                error = %e,
                "[sandbox:local] failed to remove per-call dir"
            ),
        }
    }
}

/// Execute via the OS-level `cwd_jail` backend (Landlock/Seatbelt/AppContainer).
///
/// Output capture: some OS backends (macOS Seatbelt) rebuild the command
/// internally and don't forward piped stdio settings. We capture output
/// by wrapping the command to redirect stdout/stderr into a fresh per-call
/// directory under the core's state dir (never the user's project, #6961),
/// grant the jail write access to that directory for this spawn only, and
/// read the files back after exit.
async fn execute_local_jail(
    policy: &SandboxPolicy,
    command: &str,
    working_dir: &Path,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
    explicitly_requested: bool,
) -> anyhow::Result<SandboxExecResult> {
    let mut jail = Jail::new(&policy.workspace_root, "sandbox.agent");
    if !policy.allow_network {
        jail = jail.deny_net();
    }
    for ro in &policy.read_only_mounts {
        jail = jail.add_read_only(ro);
    }
    for rw in &policy.read_write_mounts {
        jail = jail.add_read_write(rw);
    }

    let capture = CallDir::create(sandbox_capture_root(&policy.state_dir), "capture")?;
    jail = jail.add_read_write(&capture.path);
    // `/tmp` is not granted, so `mktemp`, compilers and package managers get a
    // private writable TMPDIR instead; it is removed when the call ends.
    let scratch = CallDir::create(sandbox_scratch_root(&policy.state_dir), "scratch")?;
    jail = jail.add_read_write(&scratch.path);
    let stdout_file = capture.stdout();
    let stderr_file = capture.stderr();
    let caller_sets_tmpdir = extra_env.contains_key(std::ffi::OsStr::new("TMPDIR"));
    let caller_sets_temp = extra_env.contains_key(std::ffi::OsStr::new("TEMP"));
    let caller_sets_tmp = extra_env.contains_key(std::ffi::OsStr::new("TMP"));
    // Platform-aware output-capture wrap: `{ … ; } > … 2> …` on sh/bash,
    // trailing `> … 2> …` on cmd.exe (no brace grouping). Shell binary is
    // picked by `platform_shell` so this path is Windows-safe (#4705).
    let wrapped = platform_shell::wrap_with_output_redirection(command, &stdout_file, &stderr_file);
    let mut cmd = platform_shell::build_std_command(&wrapped);
    cmd.current_dir(working_dir);
    cmd.env_clear();
    for var in SANDBOX_ENV_PASSTHROUGH {
        if let Ok(val) = std::env::var(var) {
            if val.is_empty() {
                anyhow::bail!("sandbox passthrough environment variable {var} is empty");
            }
            cmd.env(var, val);
        }
    }
    platform_shell::forward_windows_bootstrap_env_std(&mut cmd)?;
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    // Keep every Windows spelling of the temporary directory inside this
    // per-call grant. `TEMP`/`TMP` are the variables used by Windows tools;
    // `TMPDIR` covers Unix-oriented tools running on the same host.
    if !caller_sets_tmpdir {
        cmd.env("TMPDIR", &scratch.path);
    }
    if !caller_sets_temp {
        cmd.env("TEMP", &scratch.path);
    }
    if !caller_sets_tmp {
        cmd.env("TMP", &scratch.path);
    }

    let os_backend = cwd_jail::default_backend();
    if explicitly_requested && (os_backend.name() != "landlock" || !os_backend.is_available()) {
        anyhow::bail!("Landlock sandbox was explicitly requested but is unavailable");
    }
    let spawn_result = if os_backend.is_available() {
        cwd_jail::spawn_with(os_backend.as_ref(), &jail, cmd)
    } else {
        tracing::debug!("[sandbox:local] OS backend unavailable, using noop");
        cwd_jail::spawn_with(&NoopBackend, &jail, cmd)
    };

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
                                // The jailed command is a shell with a
                                // pipeline behind it: signal its whole group,
                                // then the child itself, and reap it so the
                                // timeout report is true of every process.
                                crate::tools::timeout::kill_process_group(child.id());
                                let _ = child.kill();
                                let _ = child.wait();
                                return Ok((-1, true));
                            }
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        Err(e) => return Err(e),
                    }
                }
            })
            .await??;

            let stdout = std::fs::read_to_string(&stdout_file).unwrap_or_default();
            let stderr_content = std::fs::read_to_string(&stderr_file).unwrap_or_default();
            drop(capture);
            drop(scratch);

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
        Err(e) => anyhow::bail!("Failed to spawn jailed process: {e}"),
    }
}

/// Check whether a tool operation is an elevated op that must run on the
/// host even when the session is sandboxed.
pub fn is_elevated_op(tool_name: &str) -> bool {
    ELEVATED_TOOLS.contains(&tool_name)
}

fn host_local_jail_grants(runtime: &RuntimeConfig) -> JailGrants {
    local_jail_grants_with_home(runtime, crate::core::runtime::is_saas(), || {
        dirs::home_dir()
    })
}

// Shared by initial selection and an explicit backend switch. SaaS callers
// have no operator-home grants, even if the helper is reached directly.
fn local_jail_grants_with_home(
    runtime: &RuntimeConfig,
    saas: bool,
    home: impl FnOnce() -> Option<PathBuf>,
) -> JailGrants {
    let home = if saas { None } else { home() };
    resolve_local_jail_grants(home.as_deref(), &runtime.local_jail)
}

#[cfg(test)]
#[path = "ops_host_grants_tests.rs"]
mod host_grants_tests;

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
