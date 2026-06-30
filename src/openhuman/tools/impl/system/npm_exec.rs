//! `npm_exec` — invoke the npm CLI through the managed (or system) Node.js
//! toolchain.
//!
//! Thin wrapper over `npm <subcommand> <args...>` that piggybacks on
//! [`crate::openhuman::javascript::NodeBootstrap`] for binary resolution.
//! Same security posture as
//! [`crate::openhuman::tools::impl::system::shell::ShellTool`] and
//! [`crate::openhuman::tools::impl::system::node_exec::NodeExecTool`]:
//!
//! * Host env is cleared before spawning; only functional vars (`HOME`,
//!   `TERM`, `LANG`, …) are forwarded.
//! * `PATH` is rebuilt with the resolved bin dir prepended so `npm`'s own
//!   `node`/`corepack` lookups hit the managed toolchain first.
//! * Rate limits + action budget tracking piggyback on `SecurityPolicy`.
//!
//! The `subcommand` parameter is required and cannot contain shell
//! metacharacters (guarded server-side). Free-form args go through
//! POSIX-safe single-quoting.

use crate::openhuman::agent::host_runtime::RuntimeAdapter;
use crate::openhuman::javascript::NodeBootstrap;
use crate::openhuman::security::{CommandClass, GateDecision, SecurityPolicy};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult, ToolTimeout};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// Absolute ceiling callers can request via `timeout_secs`. There is **no**
/// default timeout — `npm install`/build steps on a cold cache or slow network
/// legitimately take minutes and must not be hard-killed by a default cap
/// (issue #4023). A deadline applies only when `timeout_secs` is supplied.
const NPM_TIMEOUT_MAX_SECS: u64 = 1800;
/// Output cap per stream (1 MB).
const MAX_OUTPUT_BYTES: usize = 1_048_576;
/// Env allow-list — matches the shell / node_exec tools.
const SAFE_ENV_VARS: &[&str] = &[
    "HOME",
    "TERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "USER",
    "SHELL",
    "TMPDIR",
    // Windows process creation and child command lookup need these after env_clear().
    // PATH is rebuilt separately with the managed Node bin dir prepended.
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

/// Subcommands we outright refuse to run. These either break the managed
/// cache (`uninstall` of tooling bundled with the install) or perform
/// write actions outside the workspace (`publish` to a registry, `adduser`
/// / `login` / `logout` which mutate `~/.npmrc`).
const DISALLOWED_SUBCOMMANDS: &[&str] = &[
    "publish",
    "unpublish",
    "adduser",
    "login",
    "logout",
    "token",
    "star",
    "unstar",
    "owner",
    "access",
    "team",
    "hook",
    "profile",
];

/// `npm_exec` — run npm subcommands (install, run, ci, test, …).
pub struct NpmExecTool {
    security: Arc<SecurityPolicy>,
    runtime: Arc<dyn RuntimeAdapter>,
    bootstrap: Arc<NodeBootstrap>,
}

impl NpmExecTool {
    pub fn new(
        security: Arc<SecurityPolicy>,
        runtime: Arc<dyn RuntimeAdapter>,
        bootstrap: Arc<NodeBootstrap>,
    ) -> Self {
        Self {
            security,
            runtime,
            bootstrap,
        }
    }
}

#[async_trait]
impl Tool for NpmExecTool {
    fn name(&self) -> &str {
        "npm_exec"
    }

    fn description(&self) -> &str {
        "Run an npm subcommand (install, ci, run, test, exec, …) in the workspace. Dangerous registry/auth commands (publish, login, adduser, token, …) are blocked."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "subcommand": {
                    "type": "string",
                    "description": "npm subcommand, e.g. `install`, `ci`, `run`, `test`, `exec`."
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Arguments appended after the subcommand (e.g. [\"build\"] for `npm run build`)."
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional sub-directory (relative to workspace) to run npm in. Defaults to the workspace root."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Optional wall-clock timeout (seconds) before npm is killed. No timeout by default — installs/builds run to completion. Capped at 1800s; 0 disables."
                }
            },
            "required": ["subcommand"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    /// `npm_exec` runs installs/builds that legitimately take a long time, so it
    /// runs unbounded unless the caller passes an explicit `timeout_secs`
    /// (capped at [`NPM_TIMEOUT_MAX_SECS`]).
    fn timeout_policy(&self, args: &serde_json::Value) -> ToolTimeout {
        npm_timeout_policy(args)
    }

    /// npm subcommands run arbitrary scripts (`run`/`exec`/lifecycle hooks) →
    /// the `Write` bucket, so ask-before-edit routes through the human approval
    /// gate and read-only `execute` refuses below. Previously `npm_exec`
    /// bypassed the gate (only the rate limiter applied).
    fn external_effect_with_args(&self, _args: &serde_json::Value) -> bool {
        self.security.gate_decision(CommandClass::Write) == GateDecision::Prompt
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let subcommand = match args.get("subcommand").and_then(|v| v.as_str()) {
            Some(s) => s.trim().to_string(),
            None => {
                return Ok(ToolResult::error(
                    "npm_exec requires a `subcommand` (e.g. install, ci, run).",
                ));
            }
        };
        if subcommand.is_empty() {
            return Ok(ToolResult::error("npm_exec `subcommand` cannot be empty"));
        }
        if !is_sane_subcommand(&subcommand) {
            return Ok(ToolResult::error(format!(
                "npm_exec rejected subcommand {subcommand:?}: only alphanumeric/._- characters allowed"
            )));
        }
        if DISALLOWED_SUBCOMMANDS
            .iter()
            .any(|d| d.eq_ignore_ascii_case(&subcommand))
        {
            return Ok(ToolResult::error(format!(
                "npm_exec refuses to run `npm {subcommand}` — registry/auth mutations are blocked"
            )));
        }

        let extra_args: Vec<String> = args
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let cwd_override = args.get("cwd").and_then(|v| v.as_str()).map(str::to_string);

        // No default deadline — only a caller-supplied `timeout_secs` (capped)
        // bounds the run. `None` ⇒ run to completion.
        let explicit_timeout = crate::openhuman::tool_timeout::explicit_call_timeout_duration(
            args.get("timeout_secs").and_then(|v| v.as_u64()),
            NPM_TIMEOUT_MAX_SECS,
        );

        // Read-only mode performs no acts. npm runs arbitrary scripts, so it
        // must refuse here — it previously skipped the autonomy check entirely.
        if !self.security.can_act() {
            return Ok(ToolResult::error(
                "[policy-blocked] Action blocked: the agent is in read-only mode and cannot run npm.",
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

        let cwd = match resolve_cwd(&self.security.action_dir, cwd_override.as_deref()) {
            Ok(p) => p,
            Err(msg) => return Ok(ToolResult::error(msg)),
        };

        let resolved = match self.bootstrap.resolve().await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "[npm_exec] failed to resolve node runtime");
                return Ok(ToolResult::error(format!(
                    "Node.js runtime unavailable: {e}"
                )));
            }
        };

        tracing::info!(
            version = %resolved.version,
            source = ?resolved.source,
            npm_bin = %resolved.npm_bin.display(),
            subcommand = %subcommand,
            "[npm_exec] starting invocation"
        );

        let mut parts: Vec<String> = Vec::with_capacity(extra_args.len() + 2);
        parts.push(shell_quote(&resolved.npm_bin.to_string_lossy()));
        parts.push(shell_quote(&subcommand));
        for a in &extra_args {
            parts.push(shell_quote(a));
        }
        let command = parts.join(" ");

        // When the agent's sandbox mode is `Sandboxed`, route execution
        // through the sandbox backend (Docker / OS-level `cwd_jail` /
        // documented noop) instead of the native runtime path. Mirrors
        // the wiring in `ShellTool::run_with_security` (PR #3261) so
        // npm_exec gets the same isolation guarantees as shell. The
        // security/rate-limit checks above still apply.
        if matches!(
            crate::openhuman::agent::harness::current_sandbox_mode(),
            Some(crate::openhuman::agent::harness::definition::SandboxMode::Sandboxed)
        ) {
            return Ok(self
                .run_sandboxed(&command, &cwd, &resolved.bin_dir, explicit_timeout)
                .await);
        }

        let mut cmd = match self.runtime.build_shell_command(&command, &cwd) {
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

        for var in SAFE_ENV_VARS {
            if let Ok(val) = std::env::var(var) {
                cmd.env(var, val);
            }
        }

        // Bounded only when the caller asked for a deadline; otherwise run to
        // completion (no harness/tool timeout on long installs/builds).
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
                    // Surface exit code + both streams so the agent can diagnose
                    // the failure instead of re-running it (#4095).
                    Ok(super::command_output::command_failure(
                        output.status.code(),
                        &stdout,
                        &stderr,
                    ))
                }
            }
            Ok(Err(e)) => Ok(ToolResult::error(format!("Failed to execute npm: {e}"))),
            Err(_) => Ok(ToolResult::error(format!(
                "npm_exec timed out after {}s and was killed",
                explicit_timeout.map(|d| d.as_secs()).unwrap_or(0)
            ))),
        }
    }
}

impl NpmExecTool {
    /// Execute an npm command through the sandbox backend. Called from
    /// `execute()` when the agent's `SandboxMode` is `Sandboxed`.
    ///
    /// Mirrors `ShellTool::run_sandboxed` and `NodeExecTool::run_sandboxed`.
    /// The sandbox policy is resolved from the current `RuntimeConfig` and
    /// rooted at `security.action_dir` — note that the actual child-process
    /// `working_dir` may be a sub-path of `action_dir` (the resolved `cwd`
    /// from `cwd_override`), kept consistent with the unsandboxed path.
    async fn run_sandboxed(
        &self,
        command: &str,
        cwd: &std::path::Path,
        bin_dir: &std::path::Path,
        timeout: Option<Duration>,
    ) -> ToolResult {
        use crate::openhuman::sandbox;

        // Sandbox backends require a finite deadline. When the caller did not
        // request one, use a generous effective-unbounded cap (24h) — long
        // enough not to kill a legitimate install/build, finite enough to
        // eventually reclaim a wedged sandbox process. The native path runs
        // truly unbounded.
        let effective = timeout.unwrap_or_else(|| {
            Duration::from_secs(crate::openhuman::tool_timeout::SANDBOX_UNBOUNDED_CAP_SECS)
        });

        // Load the live `RuntimeConfig` so `resolve_sandbox_policy` derives
        // the right backend (Docker / local / noop) from the operator's
        // configuration instead of the unconfigured `RuntimeConfig::default()`.
        // Falls back to defaults with a warning if the config load fails —
        // a failed config read shouldn't block tool execution. (CodeRabbit
        // finding on PR #3309.)
        let runtime_cfg = match crate::openhuman::config::ops::load_config_with_timeout().await {
            Ok(cfg) => cfg.runtime,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "[npm_exec] failed to load live RuntimeConfig — falling back to defaults"
                );
                crate::openhuman::config::RuntimeConfig::default()
            }
        };
        // `is_remote_session = false` matches `ShellTool::run_sandboxed`'s
        // current behavior (PR #3261). Threading the real session origin
        // through requires a new `tokio::task_local!` next to
        // `CURRENT_AGENT_SANDBOX_MODE` and is the same gap across all three
        // shell-family tools; tracked separately so it can be fixed uniformly.
        let policy = sandbox::resolve_sandbox_policy(
            crate::openhuman::agent::harness::definition::SandboxMode::Sandboxed,
            &self.security.action_dir,
            &runtime_cfg,
            false,
        );

        tracing::debug!(
            backend = ?policy.backend,
            runtime_kind = ?runtime_cfg.kind,
            "[npm_exec] routing to sandbox backend"
        );

        // Forward the managed Node.js bin dir on PATH so npm child invocations
        // (e.g. `npm run` spawning user scripts) resolve `node`/`npx`
        // consistently with the unsandboxed path.
        let mut extra_env = std::collections::HashMap::new();
        let host_path = std::env::var("PATH").unwrap_or_default();
        let sep = if cfg!(windows) { ";" } else { ":" };
        let prepended = if host_path.is_empty() {
            bin_dir.to_string_lossy().into_owned()
        } else {
            format!("{}{}{}", bin_dir.display(), sep, host_path)
        };
        extra_env.insert("PATH".to_string(), prepended);

        match sandbox::execute_in_sandbox(&policy, command, cwd, extra_env, effective).await {
            Ok(result) => {
                if result.timed_out {
                    ToolResult::error(format!(
                        "npm_exec timed out after {}s and was killed",
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

/// Resolve the wall-clock policy for an `npm_exec` call from its args.
///
/// No `timeout_secs` (or `0`) ⇒ run unbounded; a positive value ⇒ enforce it,
/// clamped to [`NPM_TIMEOUT_MAX_SECS`]. Extracted from
/// [`NpmExecTool::timeout_policy`] so it is unit-testable without a bootstrap.
fn npm_timeout_policy(args: &serde_json::Value) -> ToolTimeout {
    match args.get("timeout_secs").and_then(|v| v.as_u64()) {
        None | Some(0) => ToolTimeout::Unbounded,
        Some(secs) => ToolTimeout::Secs(secs.min(NPM_TIMEOUT_MAX_SECS)),
    }
}

/// POSIX-safe single-quote escaping (mirrors the helper in `node_exec`).
/// Wraps `s` in `'…'`, turning any embedded single-quote into `'\''` so no
/// shell metacharacter can escape the quoted string.
fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

/// Subcommands must be plain identifiers (`install`, `run`, `ci`, `exec`,
/// `test:watch`) — never a command substitution or redirection payload.
fn is_sane_subcommand(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'))
}

/// Resolve an optional `cwd` override against the workspace. Rejects any
/// path that escapes the workspace via `..` or absolute components.
fn resolve_cwd(
    workspace: &std::path::Path,
    override_path: Option<&str>,
) -> Result<std::path::PathBuf, String> {
    match override_path {
        None => Ok(workspace.to_path_buf()),
        Some(raw) => {
            let raw = raw.trim();
            if raw.is_empty() || raw == "." {
                return Ok(workspace.to_path_buf());
            }
            let candidate = std::path::Path::new(raw);
            if candidate.is_absolute() {
                return Err(format!(
                    "npm_exec `cwd` must be relative to workspace; got absolute path {raw:?}"
                ));
            }
            if candidate.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir | std::path::Component::Prefix(_)
                )
            }) {
                return Err(format!(
                    "npm_exec `cwd` must not escape workspace; got {raw:?}"
                ));
            }
            Ok(workspace.join(candidate))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn absolute_sample() -> &'static str {
        if cfg!(windows) {
            "C:\\Windows\\System32"
        } else {
            "/etc"
        }
    }

    #[test]
    fn npm_timeout_policy_unbounded_by_default() {
        assert_eq!(npm_timeout_policy(&json!({})), ToolTimeout::Unbounded);
        assert_eq!(
            npm_timeout_policy(&json!({"timeout_secs": 0})),
            ToolTimeout::Unbounded
        );
    }

    #[test]
    fn npm_timeout_policy_enforces_and_caps_explicit() {
        assert_eq!(
            npm_timeout_policy(&json!({"timeout_secs": 300})),
            ToolTimeout::Secs(300)
        );
        assert_eq!(
            npm_timeout_policy(&json!({"timeout_secs": 99999})),
            ToolTimeout::Secs(NPM_TIMEOUT_MAX_SECS)
        );
    }

    #[test]
    fn is_sane_subcommand_accepts_common_npm_verbs() {
        for v in &[
            "install",
            "ci",
            "run",
            "exec",
            "test",
            "test:watch",
            "run-script",
        ] {
            assert!(is_sane_subcommand(v), "{v} should be accepted");
        }
    }

    #[test]
    fn is_sane_subcommand_rejects_metacharacters() {
        for v in &["install; rm -rf /", "run && echo", "|cat", "$(whoami)", ""] {
            assert!(!is_sane_subcommand(v), "{v} should be rejected");
        }
    }

    #[test]
    fn resolve_cwd_defaults_to_workspace() {
        let ws = std::path::Path::new("/tmp/ws");
        assert_eq!(resolve_cwd(ws, None).unwrap(), ws);
        assert_eq!(resolve_cwd(ws, Some("")).unwrap(), ws);
        assert_eq!(resolve_cwd(ws, Some(".")).unwrap(), ws);
    }

    #[test]
    fn resolve_cwd_rejects_absolute_and_parent() {
        let ws = std::path::Path::new("/tmp/ws");
        assert!(resolve_cwd(ws, Some(absolute_sample())).is_err());
        assert!(resolve_cwd(ws, Some("../other")).is_err());
        assert!(resolve_cwd(ws, Some("sub/../../../etc")).is_err());
    }

    #[test]
    fn resolve_cwd_allows_relative_subdir() {
        let ws = std::path::Path::new("/tmp/ws");
        let got = resolve_cwd(ws, Some("app")).unwrap();
        assert_eq!(got, std::path::PathBuf::from("/tmp/ws/app"));
    }

    #[test]
    fn safe_env_vars_include_windows_process_essentials() {
        for var in ["SystemRoot", "COMSPEC", "PATHEXT", "TEMP", "USERPROFILE"] {
            assert!(
                SAFE_ENV_VARS.contains(&var),
                "{var} must be forwarded for Windows child processes"
            );
        }
    }
}
