//! `python_exec` — execute Python 3 via the managed (or system) interpreter.
//!
//! Sibling to [`node_exec`](super::node_exec) with identical security gates and
//! env hygiene; the command is pinned to the `python` binary resolved by
//! [`crate::openhuman::runtime::python::PythonBootstrap`].
//!
//! Two input modes:
//!
//! | Mode          | Params                                    | Resulting invocation           |
//! |---------------|-------------------------------------------|--------------------------------|
//! | Inline code   | `inline_code: "print(1+1)"`               | `python -c '<code>'`           |
//! | Script path   | `script_path: "scripts/run.py"`, `args`   | `python <path> <args...>`      |
//!
//! Inline code routes through the shared runtime pool (#5106) when enabled — a
//! warm, bounded set of `python` workers replaces one interpreter child per
//! call — and transparently falls back to a per-call spawn otherwise. Script
//! paths and sandboxed runs always use the per-call spawn.

use crate::openhuman::agent::host_runtime::RuntimeAdapter;
use crate::openhuman::runtime::python::PythonBootstrap;
use crate::openhuman::security::{CommandClass, GateDecision, SecurityPolicy};
use crate::openhuman::tools::traits::{
    PermissionLevel, Tool, ToolCallOptions, ToolResult, ToolTimeout,
};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tinytools::ToolRunContext;

/// Absolute ceiling a caller may request via `timeout_secs`. No default timeout —
/// Python scripts legitimately take minutes; a deadline applies only when
/// `timeout_secs` is supplied. Mirrors `node_exec` (issue #4023).
const PYTHON_TIMEOUT_MAX_SECS: u64 = 1800;
/// Maximum combined stdout/stderr size (1 MB each) — same cap as shell/node.
const MAX_OUTPUT_BYTES: usize = 1_048_576;
/// Env allow-list for child processes. Matches node_exec/shell — secrets never
/// leak into spawned python processes. `PATH` gets a prepend of the managed bin
/// dir before being forwarded.
const SAFE_ENV_VARS: &[&str] = &[
    "HOME",
    "TERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "USER",
    "SHELL",
    "TMPDIR",
    "SystemRoot",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
];

/// `python_exec` — execute Python through the resolved Python 3 runtime.
pub struct PythonExecTool {
    security: Arc<SecurityPolicy>,
    runtime: Arc<dyn RuntimeAdapter>,
    bootstrap: Arc<PythonBootstrap>,
    /// Runtime-pool config + workspace, snapshotted at construction so the hot
    /// inline path never re-reads config from disk (#5106 is a perf feature).
    pool_cfg: crate::openhuman::config::RuntimePoolConfig,
    workspace_dir: std::path::PathBuf,
}

impl PythonExecTool {
    pub fn new(
        security: Arc<SecurityPolicy>,
        runtime: Arc<dyn RuntimeAdapter>,
        bootstrap: Arc<PythonBootstrap>,
        pool_cfg: crate::openhuman::config::RuntimePoolConfig,
        workspace_dir: std::path::PathBuf,
    ) -> Self {
        Self {
            security,
            runtime,
            bootstrap,
            pool_cfg,
            workspace_dir,
        }
    }
}

#[async_trait]
impl Tool for PythonExecTool {
    fn name(&self) -> &str {
        "python_exec"
    }

    fn description(&self) -> &str {
        "Execute Python 3 through the managed interpreter. Pass either `inline_code` (runs via `python -c`) or `script_path` (runs a .py file in your working directory, the action sandbox). Optional `args` forwards positional arguments to the script. Only the program's stdout/stderr is captured and returned to you — a value you do not `print` is invisible. Always print the output you need (e.g. `print(json.dumps(result))`)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "inline_code": {
                    "type": "string",
                    "description": "Python source passed to `python -c`. Mutually exclusive with script_path."
                },
                "script_path": {
                    "type": "string",
                    "description": "Path (relative to workspace) to a .py file. Mutually exclusive with inline_code."
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Positional arguments appended after the script. Ignored for inline_code."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Optional wall-clock timeout (seconds) before the process is killed. No timeout by default. Capped at 1800s; 0 disables."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn timeout_policy(&self, args: &serde_json::Value) -> ToolTimeout {
        python_timeout_policy(args)
    }

    /// Running Python is arbitrary code execution → the `Write` bucket, same as
    /// `node_exec`. In ask-before-edit this routes through the human approval
    /// gate; in Full it runs; in read-only it refuses below.
    fn external_effect_with_args(&self, _args: &serde_json::Value) -> bool {
        self.security.gate_decision(CommandClass::Write) == GateDecision::Prompt
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_in_context(args, None).await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.execute_in_context(args, context).await
    }
}

impl PythonExecTool {
    async fn execute_in_context(
        &self,
        args: serde_json::Value,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let inline_code = args
            .get("inline_code")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let script_path = args
            .get("script_path")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        let extra_args: Vec<String> = args
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let explicit_timeout = crate::openhuman::tools::timeout::explicit_call_timeout_duration(
            args.get("timeout_secs").and_then(|v| v.as_u64()),
            PYTHON_TIMEOUT_MAX_SECS,
        );

        if inline_code.is_some() == script_path.is_some() {
            return Ok(ToolResult::error(
                "python_exec requires exactly one of `inline_code` or `script_path`",
            ));
        }

        if !self.security.can_act() {
            return Ok(ToolResult::error(
                "[policy-blocked] Action blocked: the agent is in read-only mode and cannot execute code.",
            ));
        }
        if self.security.is_rate_limited() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: too many actions in the last hour",
            ));
        }
        if !self.security.record_action() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: action budget exhausted",
            ));
        }

        let resolved = match self.bootstrap.resolve().await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "[python_exec] failed to resolve python runtime");
                return Ok(ToolResult::error(format!(
                    "Python runtime unavailable: {e}"
                )));
            }
        };

        tracing::info!(
            version = %resolved.version,
            python_bin = %resolved.python_bin.display(),
            "[python_exec] starting invocation"
        );

        let path_policy = super::security_for_tool_context(&self.security, context, "python_exec");

        let command = if let Some(code) = inline_code.as_deref() {
            format!(
                "{} -c {}",
                shell_quote(&resolved.python_bin.to_string_lossy()),
                shell_quote(code)
            )
        } else if let Some(path) = script_path.as_deref() {
            let resolved_script = match resolve_script_path(&path_policy.action_dir, path) {
                Ok(p) => p,
                Err(msg) => return Ok(ToolResult::error(msg)),
            };
            let mut parts: Vec<String> = Vec::with_capacity(extra_args.len() + 2);
            parts.push(shell_quote(&resolved.python_bin.to_string_lossy()));
            parts.push(shell_quote(&resolved_script.to_string_lossy()));
            for a in &extra_args {
                parts.push(shell_quote(a));
            }
            parts.join(" ")
        } else {
            unreachable!("guarded above")
        };

        // Sandboxed agents route through the sandbox backend, mirroring node_exec.
        if matches!(
            crate::openhuman::agent::harness::current_sandbox_mode(),
            Some(crate::openhuman::agent::harness::definition::SandboxMode::Sandboxed)
        ) {
            let bin_dir = resolved
                .python_bin
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_default();
            return Ok(self
                .run_sandboxed(&path_policy, &command, &bin_dir, explicit_timeout)
                .await);
        }

        // Route inline Python through the shared runtime pool when enabled
        // (#5106). script_path and sandboxed runs keep the per-call spawn; a
        // pool infrastructure failure also transparently falls back below.
        if let Some(code) = inline_code.as_deref() {
            if let Some(result) = self
                .try_pool_inline(code, &path_policy.action_dir, explicit_timeout)
                .await
            {
                return Ok(result);
            }
        }

        let mut cmd = match self
            .runtime
            .build_shell_command(&command, &path_policy.action_dir)
        {
            Ok(cmd) => cmd,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Failed to build runtime command: {e}"
                )));
            }
        };

        cmd.env_clear();

        let host_path = std::env::var("PATH").unwrap_or_default();
        let sep = if cfg!(windows) { ";" } else { ":" };
        let prepended_path = if host_path.is_empty() {
            resolved.bin_dir.to_string_lossy().into_owned()
        } else {
            format!("{}{}{}", resolved.bin_dir.display(), sep, host_path)
        };
        cmd.env("PATH", &prepended_path);
        // Unbuffered stdio so partial output survives a kill on timeout.
        cmd.env("PYTHONUNBUFFERED", "1");

        for var in SAFE_ENV_VARS {
            if let Ok(val) = std::env::var(var) {
                cmd.env(var, val);
            }
        }

        let result = match explicit_timeout {
            Some(timeout) => tokio::time::timeout(timeout, cmd.output()).await,
            None => Ok(cmd.output().await),
        };

        match result {
            Ok(Ok(output)) => {
                let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();

                if stdout.len() > MAX_OUTPUT_BYTES {
                    stdout.truncate(crate::openhuman::util::floor_char_boundary(
                        &stdout,
                        MAX_OUTPUT_BYTES,
                    ));
                    stdout.push_str("\n... [stdout truncated at 1MB]");
                }
                if stderr.len() > MAX_OUTPUT_BYTES {
                    stderr.truncate(crate::openhuman::util::floor_char_boundary(
                        &stderr,
                        MAX_OUTPUT_BYTES,
                    ));
                    stderr.push_str("\n... [stderr truncated at 1MB]");
                }

                if output.status.success() {
                    if stderr.is_empty() {
                        Ok(ToolResult::success(stdout))
                    } else {
                        Ok(ToolResult::success(format!("{stdout}\n[stderr]\n{stderr}")))
                    }
                } else {
                    Ok(super::command_output::command_failure(
                        output.status.code(),
                        &stdout,
                        &stderr,
                    ))
                }
            }
            Ok(Err(e)) => Ok(ToolResult::error(format!("Failed to execute python: {e}"))),
            Err(_) => Ok(ToolResult::error(format!(
                "python_exec timed out after {}s and was killed",
                explicit_timeout.map(|d| d.as_secs()).unwrap_or(0)
            ))),
        }
    }

    /// Attempt to run inline Python on the shared runtime pool (#5106). Returns
    /// `Some(result)` when the pool handled the job; `None` when pooling is
    /// disabled or the pool infrastructure failed (caller falls back to spawn).
    async fn try_pool_inline(
        &self,
        code: &str,
        action_dir: &std::path::Path,
        timeout: Option<Duration>,
    ) -> Option<ToolResult> {
        if !crate::openhuman::runtime::pool::python::enabled(&self.pool_cfg) {
            return None;
        }
        match crate::openhuman::runtime::pool::python::run_inline(
            self.bootstrap.config(),
            code.to_string(),
            Some(action_dir.to_path_buf()),
            timeout,
        )
        .await
        {
            Ok(outcome) => {
                tracing::info!(
                    queue_wait_ms = outcome.queue_wait.as_millis() as u64,
                    elapsed_ms = outcome.elapsed.as_millis() as u64,
                    timed_out = outcome.timed_out,
                    "[python_exec] pool: inline job completed on a warm worker"
                );
                Some(pool_outcome_to_result(outcome, timeout))
            }
            // Job never ran → safe to fall back to a per-call spawn.
            Err(crate::openhuman::runtime::pool::PoolRunError::PreDispatch(error)) => {
                tracing::warn!(
                    error = %error,
                    "[python_exec] pool: pre-dispatch failure; falling back to legacy spawn"
                );
                None
            }
            // Load-shed: do NOT spawn (that reintroduces per-run RSS). Surface busy.
            Err(crate::openhuman::runtime::pool::PoolRunError::Saturated) => {
                tracing::warn!("[python_exec] pool: saturated; shedding load");
                Some(ToolResult::error(
                    "Python runtime pool is at capacity; retry shortly.",
                ))
            }
            // The job may already have executed → terminal, never re-run it.
            Err(crate::openhuman::runtime::pool::PoolRunError::PostDispatch(error)) => {
                tracing::warn!(
                    error = %error,
                    "[python_exec] pool: post-dispatch failure; not retried to avoid duplicate execution"
                );
                Some(ToolResult::error(format!(
                    "python_exec failed after the code was dispatched to a pooled worker; not retried to avoid running it twice: {error}"
                )))
            }
        }
    }

    /// Execute a python command through the sandbox backend. Mirrors
    /// `NodeExecTool::run_sandboxed`.
    async fn run_sandboxed(
        &self,
        security: &SecurityPolicy,
        command: &str,
        bin_dir: &std::path::Path,
        timeout: Option<Duration>,
    ) -> ToolResult {
        use crate::openhuman::sandbox;

        let effective = timeout.unwrap_or_else(|| {
            Duration::from_secs(crate::openhuman::tools::timeout::SANDBOX_UNBOUNDED_CAP_SECS)
        });

        let runtime_cfg = match crate::openhuman::config::ops::load_config_with_timeout().await {
            Ok(cfg) => cfg.runtime,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "[python_exec] failed to load live RuntimeConfig — falling back to defaults"
                );
                crate::openhuman::config::RuntimeConfig::default()
            }
        };
        let policy = sandbox::resolve_sandbox_policy(
            crate::openhuman::agent::harness::definition::SandboxMode::Sandboxed,
            &security.action_dir,
            &runtime_cfg,
            false,
        );

        let mut extra_env = std::collections::HashMap::new();
        let host_path = std::env::var("PATH").unwrap_or_default();
        let sep = if cfg!(windows) { ";" } else { ":" };
        let prepended = if host_path.is_empty() {
            bin_dir.to_string_lossy().into_owned()
        } else {
            format!("{}{}{}", bin_dir.display(), sep, host_path)
        };
        extra_env.insert("PATH".into(), prepended.into());
        extra_env.insert("PYTHONUNBUFFERED".into(), "1".into());

        match sandbox::execute_in_sandbox(
            &policy,
            command,
            &security.action_dir,
            extra_env,
            effective,
        )
        .await
        {
            Ok(result) => {
                if result.timed_out {
                    ToolResult::error(format!(
                        "python_exec timed out after {}s and was killed",
                        effective.as_secs()
                    ))
                } else if result.success() {
                    if result.stderr.is_empty() {
                        ToolResult::success(result.stdout)
                    } else {
                        ToolResult::success(format!(
                            "{}\n[stderr]\n{}",
                            result.stdout, result.stderr
                        ))
                    }
                } else {
                    super::command_output::command_failure(
                        super::command_output::sandbox_exit_code(result.exit_code),
                        &result.stdout,
                        &result.stderr,
                    )
                }
            }
            Err(e) => ToolResult::error(format!("Sandbox execution failed: {e}")),
        }
    }
}

/// Map a runtime-pool outcome onto the same `ToolResult` shape the legacy
/// `python -c` path produces.
fn pool_outcome_to_result(
    outcome: crate::openhuman::runtime::pool::PoolExecOutcome,
    timeout: Option<Duration>,
) -> ToolResult {
    if outcome.timed_out {
        return ToolResult::error(format!(
            "python_exec timed out after {}s and was killed",
            timeout.map(|d| d.as_secs()).unwrap_or(0)
        ));
    }

    let mut stdout = outcome.stdout;
    let mut stderr = outcome.stderr;
    if stdout.len() > MAX_OUTPUT_BYTES {
        stdout.truncate(crate::openhuman::util::floor_char_boundary(
            &stdout,
            MAX_OUTPUT_BYTES,
        ));
        stdout.push_str("\n... [stdout truncated at 1MB]");
    }
    if stderr.len() > MAX_OUTPUT_BYTES {
        stderr.truncate(crate::openhuman::util::floor_char_boundary(
            &stderr,
            MAX_OUTPUT_BYTES,
        ));
        stderr.push_str("\n... [stderr truncated at 1MB]");
    }

    let success = matches!(outcome.exit_code, None | Some(0));
    if success {
        if stderr.is_empty() {
            ToolResult::success(stdout)
        } else {
            ToolResult::success(format!("{stdout}\n[stderr]\n{stderr}"))
        }
    } else {
        super::command_output::command_failure(outcome.exit_code, &stdout, &stderr)
    }
}

/// Resolve the wall-clock policy for a `python_exec` call from its args.
fn python_timeout_policy(args: &serde_json::Value) -> ToolTimeout {
    match args.get("timeout_secs").and_then(|v| v.as_u64()) {
        None | Some(0) => ToolTimeout::Unbounded,
        Some(secs) => ToolTimeout::Secs(secs.min(PYTHON_TIMEOUT_MAX_SECS)),
    }
}

/// POSIX-safe single-quote escaping (mirrors node_exec::shell_quote).
fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

/// Resolve a caller-supplied `script_path` against the workspace. Rejects
/// absolute paths and any component that could escape the workspace.
fn resolve_script_path(
    workspace: &std::path::Path,
    raw: &str,
) -> Result<std::path::PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("python_exec `script_path` cannot be empty".to_string());
    }
    let candidate = std::path::Path::new(raw);
    if candidate.is_absolute() {
        return Err(format!(
            "python_exec `script_path` must be relative to workspace; got absolute {raw:?}"
        ));
    }
    if candidate.components().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::Prefix(_)
        )
    }) {
        return Err(format!(
            "python_exec `script_path` must not escape workspace; got {raw:?}"
        ));
    }
    Ok(workspace.join(candidate))
}

#[cfg(test)]
#[path = "python_exec_tests.rs"]
mod tests;
