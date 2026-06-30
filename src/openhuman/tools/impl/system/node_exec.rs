//! `node_exec` — execute JavaScript via the managed (or system) Node.js
//! toolchain.
//!
//! Sibling to [`crate::openhuman::tools::impl::system::shell::ShellTool`]: same
//! security gates, same env hygiene, but the command is pinned to the `node`
//! binary resolved by
//! [`crate::openhuman::javascript::NodeBootstrap`].
//!
//! Two input modes:
//!
//! | Mode          | Params                                   | Resulting invocation                |
//! |---------------|------------------------------------------|-------------------------------------|
//! | Inline code   | `inline_code: "console.log(1+1)"`        | `node -e '<code>'`                  |
//! | Script path   | `script_path: "scripts/run.js"`, `args`  | `node <path> <args...>`             |
//!
//! Exactly one of `inline_code` / `script_path` must be supplied. Scripts are
//! resolved relative to the workspace; paths escaping the workspace are
//! rejected by the filesystem helpers.
//!
//! The bootstrap is resolved **on first invocation**, which will download +
//! extract a managed Node.js distribution if no compatible `node` is on
//! `PATH`. Subsequent calls reuse the cached install.

use crate::openhuman::agent::host_runtime::RuntimeAdapter;
use crate::openhuman::javascript::NodeBootstrap;
use crate::openhuman::security::{CommandClass, GateDecision, SecurityPolicy};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult, ToolTimeout};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// Absolute ceiling a caller may request via `timeout_secs`. There is **no**
/// default timeout — `node_exec` runs scripts that legitimately take minutes
/// (bundlers, solvers, test runs) and must not be hard-killed by a default cap
/// (issue #4023). A deadline applies only when `timeout_secs` is supplied.
const NODE_TIMEOUT_MAX_SECS: u64 = 1800;
/// Maximum combined stdout/stderr size (1 MB each) — same cap as shell.
const MAX_OUTPUT_BYTES: usize = 1_048_576;
/// Env allow-list for child processes. Matches shell.rs — secrets never leak
/// into spawned node processes. `PATH` gets a prepend of the managed bin
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

/// `node_exec` — execute JavaScript through the resolved Node.js runtime.
pub struct NodeExecTool {
    security: Arc<SecurityPolicy>,
    runtime: Arc<dyn RuntimeAdapter>,
    bootstrap: Arc<NodeBootstrap>,
}

impl NodeExecTool {
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
impl Tool for NodeExecTool {
    fn name(&self) -> &str {
        "node_exec"
    }

    fn description(&self) -> &str {
        "Execute JavaScript through Node.js. Pass either `inline_code` (runs via `node -e`) or `script_path` (runs a file in your working directory, the action sandbox). Optional `args` forwards positional arguments to the script. Only the program's stdout/stderr is captured and returned to you — a value you do not `console.log` is invisible, and a script that exits 0 without printing returns an empty result. Always print the output you need (e.g. `console.log(JSON.stringify(result))`)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "inline_code": {
                    "type": "string",
                    "description": "JavaScript source passed to `node -e`. Mutually exclusive with script_path."
                },
                "script_path": {
                    "type": "string",
                    "description": "Path (relative to workspace) to a .js/.mjs/.cjs file. Mutually exclusive with inline_code."
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Positional arguments appended after the script. Ignored for inline_code."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Optional wall-clock timeout (seconds) before the process is killed. No timeout by default — long-running scripts run to completion. Capped at 1800s; 0 disables."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    /// `node_exec` runs scripts that legitimately take a long time, so it runs
    /// unbounded unless the caller passes an explicit `timeout_secs` (capped at
    /// [`NODE_TIMEOUT_MAX_SECS`]).
    fn timeout_policy(&self, args: &serde_json::Value) -> ToolTimeout {
        node_timeout_policy(args)
    }

    /// Running JavaScript is arbitrary code execution → the `Write` bucket. In
    /// ask-before-edit this routes through the human approval gate; in Full it
    /// runs; in read-only `execute` refuses below. Previously `node_exec`
    /// bypassed the gate entirely — only the rate limiter stood in the way.
    fn external_effect_with_args(&self, _args: &serde_json::Value) -> bool {
        self.security.gate_decision(CommandClass::Write) == GateDecision::Prompt
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
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

        // No default deadline — only the caller-supplied `timeout_secs` (capped)
        // bounds the run. `None` ⇒ run to completion.
        let explicit_timeout = crate::openhuman::tool_timeout::explicit_call_timeout_duration(
            args.get("timeout_secs").and_then(|v| v.as_u64()),
            NODE_TIMEOUT_MAX_SECS,
        );

        if inline_code.is_some() == script_path.is_some() {
            return Ok(ToolResult::error(
                "node_exec requires exactly one of `inline_code` or `script_path`",
            ));
        }

        // Read-only mode performs no acts. `node_exec` runs arbitrary code, so
        // it must refuse here — it previously skipped the autonomy check
        // entirely (only the rate limiter applied), letting `node -e '…'` run
        // even in read-only mode.
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
                tracing::error!(error = %e, "[node_exec] failed to resolve node runtime");
                return Ok(ToolResult::error(format!(
                    "Node.js runtime unavailable: {e}"
                )));
            }
        };

        tracing::info!(
            version = %resolved.version,
            source = ?resolved.source,
            node_bin = %resolved.node_bin.display(),
            "[node_exec] starting invocation"
        );

        let command = if let Some(code) = inline_code.as_deref() {
            format!(
                "{} -e {}",
                shell_quote(&resolved.node_bin.to_string_lossy()),
                shell_quote(code)
            )
        } else if let Some(path) = script_path.as_deref() {
            let resolved_script = match resolve_script_path(&self.security.action_dir, path) {
                Ok(p) => p,
                Err(msg) => return Ok(ToolResult::error(msg)),
            };
            let mut parts: Vec<String> = Vec::with_capacity(extra_args.len() + 2);
            parts.push(shell_quote(&resolved.node_bin.to_string_lossy()));
            parts.push(shell_quote(&resolved_script.to_string_lossy()));
            // `extra_args` are opaque positional arguments forwarded to the
            // script. They are shell-quoted below so no shell metacharacter
            // can escape, but we do NOT treat them as workspace paths — the
            // script itself is responsible for any path validation it does
            // on its own arguments.
            for a in &extra_args {
                parts.push(shell_quote(a));
            }
            parts.join(" ")
        } else {
            unreachable!("guarded above")
        };

        // When the agent's sandbox mode is `Sandboxed`, route execution
        // through the sandbox backend (Docker / OS-level `cwd_jail` /
        // documented noop) instead of the native runtime path. Mirrors
        // the wiring in `ShellTool::run_with_security` (PR #3261) so
        // node_exec gets the same isolation guarantees as shell. The
        // security/rate-limit checks above still apply.
        if matches!(
            crate::openhuman::agent::harness::current_sandbox_mode(),
            Some(crate::openhuman::agent::harness::definition::SandboxMode::Sandboxed)
        ) {
            return Ok(self
                .run_sandboxed(&command, &resolved.bin_dir, explicit_timeout)
                .await);
        }

        let mut cmd = match self
            .runtime
            .build_shell_command(&command, &self.security.action_dir)
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

        for var in SAFE_ENV_VARS {
            if let Ok(val) = std::env::var(var) {
                cmd.env(var, val);
            }
        }

        // Bounded only when the caller asked for a deadline; otherwise run to
        // completion (no harness/tool timeout on long scripts).
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
            Ok(Err(e)) => Ok(ToolResult::error(format!("Failed to execute node: {e}"))),
            Err(_) => Ok(ToolResult::error(format!(
                "node_exec timed out after {}s and was killed",
                explicit_timeout.map(|d| d.as_secs()).unwrap_or(0)
            ))),
        }
    }
}

impl NodeExecTool {
    /// Execute a node command through the sandbox backend. Called from
    /// `execute()` when the agent's `SandboxMode` is `Sandboxed`.
    ///
    /// Mirrors `ShellTool::run_sandboxed`. The sandbox policy is resolved
    /// from the current `RuntimeConfig` and rooted at
    /// `security.action_dir`; on platforms without a real `cwd_jail`
    /// backend the local backend falls back to a documented noop with
    /// the in-Rust path-hardening guards from `SecurityPolicy` still
    /// applying (see CLAUDE.md "Action sandbox vs internal workspace").
    async fn run_sandboxed(
        &self,
        command: &str,
        bin_dir: &std::path::Path,
        timeout: Option<Duration>,
    ) -> ToolResult {
        use crate::openhuman::sandbox;

        // Sandbox backends require a finite deadline. When the caller did not
        // request one, use a generous effective-unbounded cap (24h) so a
        // legitimately long script isn't killed while still bounding a wedged
        // sandbox process. The native (non-sandboxed) path runs truly unbounded.
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
                    "[node_exec] failed to load live RuntimeConfig — falling back to defaults"
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
            "[node_exec] routing to sandbox backend"
        );

        // Forward the managed Node.js bin dir on PATH so the child node
        // process can resolve `node`, `npm`, `npx`, `corepack` consistently
        // with the unsandboxed path.
        let mut extra_env = std::collections::HashMap::new();
        let host_path = std::env::var("PATH").unwrap_or_default();
        let sep = if cfg!(windows) { ";" } else { ":" };
        let prepended = if host_path.is_empty() {
            bin_dir.to_string_lossy().into_owned()
        } else {
            format!("{}{}{}", bin_dir.display(), sep, host_path)
        };
        extra_env.insert("PATH".to_string(), prepended);

        match sandbox::execute_in_sandbox(
            &policy,
            command,
            &self.security.action_dir,
            extra_env,
            effective,
        )
        .await
        {
            Ok(result) => {
                if result.timed_out {
                    ToolResult::error(format!(
                        "node_exec timed out after {}s and was killed",
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

/// Resolve the wall-clock policy for a `node_exec` call from its args.
///
/// No `timeout_secs` (or `0`) ⇒ run unbounded; a positive value ⇒ enforce it,
/// clamped to [`NODE_TIMEOUT_MAX_SECS`]. Extracted from
/// [`NodeExecTool::timeout_policy`] so it is unit-testable without a bootstrap.
fn node_timeout_policy(args: &serde_json::Value) -> ToolTimeout {
    match args.get("timeout_secs").and_then(|v| v.as_u64()) {
        None | Some(0) => ToolTimeout::Unbounded,
        Some(secs) => ToolTimeout::Secs(secs.min(NODE_TIMEOUT_MAX_SECS)),
    }
}

/// POSIX-safe single-quote escaping. Wraps `s` in `'…'`, turning any embedded
/// single-quote into the four-char sequence `'\''`. Node bin paths and user
/// code pass through untouched semantically, but no shell metacharacter can
/// escape the quoted string.
fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

/// Resolve a caller-supplied `script_path` against the workspace. Mirrors
/// `npm_exec::resolve_cwd` — rejects absolute paths and any component that
/// could escape the workspace (`..`, Windows drive prefixes). Scripts
/// themselves must live inside the workspace.
fn resolve_script_path(
    workspace: &std::path::Path,
    raw: &str,
) -> Result<std::path::PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("node_exec `script_path` cannot be empty".to_string());
    }
    let candidate = std::path::Path::new(raw);
    if candidate.is_absolute() {
        return Err(format!(
            "node_exec `script_path` must be relative to workspace; got absolute {raw:?}"
        ));
    }
    if candidate.components().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::Prefix(_)
        )
    }) {
        return Err(format!(
            "node_exec `script_path` must not escape workspace; got {raw:?}"
        ));
    }
    Ok(workspace.join(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn absolute_sample() -> &'static str {
        if cfg!(windows) {
            "C:\\Windows\\System32\\drivers\\etc\\hosts"
        } else {
            "/etc/passwd"
        }
    }

    #[test]
    fn shell_quote_wraps_plain_strings() {
        assert_eq!(shell_quote("node"), "'node'");
        assert_eq!(shell_quote("/opt/bin/node"), "'/opt/bin/node'");
    }

    #[test]
    fn node_timeout_policy_unbounded_by_default() {
        // No timeout_secs (or explicit 0) ⇒ run to completion.
        assert_eq!(node_timeout_policy(&json!({})), ToolTimeout::Unbounded);
        assert_eq!(
            node_timeout_policy(&json!({"timeout_secs": 0})),
            ToolTimeout::Unbounded
        );
    }

    #[test]
    fn node_timeout_policy_enforces_and_caps_explicit() {
        assert_eq!(
            node_timeout_policy(&json!({"timeout_secs": 120})),
            ToolTimeout::Secs(120)
        );
        // Clamped to the 1800s ceiling.
        assert_eq!(
            node_timeout_policy(&json!({"timeout_secs": 99999})),
            ToolTimeout::Secs(NODE_TIMEOUT_MAX_SECS)
        );
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(
            shell_quote("console.log('hi')"),
            "'console.log('\\''hi'\\'')'"
        );
    }

    #[test]
    fn shell_quote_neutralises_metacharacters() {
        // $, backticks, && — all inert once wrapped in single quotes.
        assert_eq!(shell_quote("$(rm -rf /)"), "'$(rm -rf /)'");
        assert_eq!(shell_quote("a && b"), "'a && b'");
    }

    #[test]
    fn resolve_script_path_rejects_empty() {
        let ws = std::path::Path::new("/ws");
        assert!(resolve_script_path(ws, "").is_err());
        assert!(resolve_script_path(ws, "   ").is_err());
    }

    #[test]
    fn resolve_script_path_rejects_absolute() {
        let ws = std::path::Path::new("/ws");
        assert!(resolve_script_path(ws, absolute_sample()).is_err());
    }

    #[test]
    fn resolve_script_path_rejects_parent_dir() {
        let ws = std::path::Path::new("/ws");
        assert!(resolve_script_path(ws, "../evil.js").is_err());
        assert!(resolve_script_path(ws, "scripts/../../evil.js").is_err());
    }

    #[test]
    fn resolve_script_path_accepts_relative_subdir() {
        let ws = std::path::Path::new("/ws");
        let resolved = resolve_script_path(ws, "scripts/run.js").unwrap();
        assert_eq!(resolved, std::path::Path::new("/ws/scripts/run.js"));
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

    /// Regression guard for #3238.
    ///
    /// `node_exec` resolves caller-supplied `script_path` values against
    /// `security.action_dir` (the agent's writable sandbox), never
    /// `security.workspace_dir` (internal product state). If a future
    /// refactor changes `NodeExecTool::execute` to pass
    /// `&self.security.workspace_dir` to `resolve_script_path`, scripts
    /// would resolve into the internal denylist instead of the action
    /// sandbox, which is exactly the action/internal split that
    /// PR #3074 prevents.
    ///
    /// The behavioural end-to-end test for the CWD plumbing lives in
    /// `shell.rs` (`shell_pwd_returns_action_dir_not_workspace_dir`) —
    /// `node_exec` shares the same `runtime.build_shell_command(&command,
    /// &self.security.action_dir)` call site, and the source-grep guard
    /// in `shell.rs` (`shell_family_tools_route_cwd_through_action_dir`)
    /// covers all three system tools. This test pins the script-resolution
    /// contract specifically for `node_exec` by exercising
    /// `resolve_script_path` against an `action_dir` distinct from
    /// `workspace_dir`.
    #[test]
    fn resolve_script_path_targets_action_dir_not_workspace_dir() {
        let action_dir = std::path::Path::new("/tmp/action-sandbox-3238");
        let workspace_dir = std::path::Path::new("/tmp/internal-workspace-3238");

        let resolved = resolve_script_path(action_dir, "scripts/run.js")
            .expect("relative script under action_dir must resolve");
        assert_eq!(
            resolved,
            action_dir.join("scripts/run.js"),
            "script_path must resolve under action_dir, not workspace_dir (see #3238)"
        );
        assert!(
            resolved.starts_with(action_dir),
            "resolved path must be under action_dir; got {}",
            resolved.display()
        );
        assert!(
            !resolved.starts_with(workspace_dir),
            "resolved path leaked into workspace_dir; got {}",
            resolved.display()
        );
    }
}
