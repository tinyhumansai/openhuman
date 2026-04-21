//! `npm_exec` — invoke the npm CLI through the managed (or system) Node.js
//! toolchain.
//!
//! Thin wrapper over `npm <subcommand> <args...>` that piggybacks on
//! [`crate::openhuman::node_runtime::NodeBootstrap`] for binary resolution.
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
use crate::openhuman::node_runtime::NodeBootstrap;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// Default wall-clock budget for an npm invocation. `npm install` on a cold
/// cache can legitimately take several minutes on slow networks.
const NPM_TIMEOUT_SECS: u64 = 600;
/// Absolute ceiling callers can request via `timeout_secs`.
const NPM_TIMEOUT_MAX_SECS: u64 = 1800;
/// Output cap per stream (1 MB).
const MAX_OUTPUT_BYTES: usize = 1_048_576;
/// Env allow-list — matches the shell / node_exec tools.
const SAFE_ENV_VARS: &[&str] = &[
    "HOME", "TERM", "LANG", "LC_ALL", "LC_CTYPE", "USER", "SHELL", "TMPDIR",
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
                    "description": "Optional override for the default 600s timeout. Capped at 1800s."
                }
            },
            "required": ["subcommand"]
        })
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

        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(NPM_TIMEOUT_SECS)
            .min(NPM_TIMEOUT_MAX_SECS);

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

        let cwd = match resolve_cwd(&self.security.workspace_dir, cwd_override.as_deref()) {
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

        let result = tokio::time::timeout(Duration::from_secs(timeout_secs), cmd.output()).await;

        match result {
            Ok(Ok(output)) => {
                let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();

                if stdout.len() > MAX_OUTPUT_BYTES {
                    stdout.truncate(stdout.floor_char_boundary(MAX_OUTPUT_BYTES));
                    stdout.push_str("\n... [stdout truncated at 1MB]");
                }
                if stderr.len() > MAX_OUTPUT_BYTES {
                    stderr.truncate(stderr.floor_char_boundary(MAX_OUTPUT_BYTES));
                    stderr.push_str("\n... [stderr truncated at 1MB]");
                }

                if output.status.success() {
                    if stderr.is_empty() {
                        Ok(ToolResult::success(stdout))
                    } else {
                        Ok(ToolResult::success(format!("{stdout}\n[stderr]\n{stderr}")))
                    }
                } else {
                    let err_msg = if stderr.is_empty() { stdout } else { stderr };
                    Ok(ToolResult::error(err_msg))
                }
            }
            Ok(Err(e)) => Ok(ToolResult::error(format!("Failed to execute npm: {e}"))),
            Err(_) => Ok(ToolResult::error(format!(
                "npm_exec timed out after {timeout_secs}s and was killed"
            ))),
        }
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
        assert!(resolve_cwd(ws, Some("/etc")).is_err());
        assert!(resolve_cwd(ws, Some("../other")).is_err());
        assert!(resolve_cwd(ws, Some("sub/../../../etc")).is_err());
    }

    #[test]
    fn resolve_cwd_allows_relative_subdir() {
        let ws = std::path::Path::new("/tmp/ws");
        let got = resolve_cwd(ws, Some("app")).unwrap();
        assert_eq!(got, std::path::PathBuf::from("/tmp/ws/app"));
    }
}
