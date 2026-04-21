//! `node_exec` — execute JavaScript via the managed (or system) Node.js
//! toolchain.
//!
//! Sibling to [`crate::openhuman::tools::impl::system::shell::ShellTool`]: same
//! security gates, same env hygiene, but the command is pinned to the `node`
//! binary resolved by
//! [`crate::openhuman::node_runtime::NodeBootstrap`].
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
use crate::openhuman::node_runtime::NodeBootstrap;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// Maximum node process wall-clock before we kill it. Longer than the shell
/// tool because `npm install` / bundler steps can legitimately exceed 60s,
/// and `node_exec` is often the launcher for those flows.
const NODE_TIMEOUT_SECS: u64 = 300;
/// Maximum combined stdout/stderr size (1 MB each) — same cap as shell.
const MAX_OUTPUT_BYTES: usize = 1_048_576;
/// Env allow-list for child processes. Matches shell.rs — secrets never leak
/// into spawned node processes. `PATH` gets a prepend of the managed bin
/// dir before being forwarded.
const SAFE_ENV_VARS: &[&str] = &[
    "HOME", "TERM", "LANG", "LC_ALL", "LC_CTYPE", "USER", "SHELL", "TMPDIR",
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
        "Execute JavaScript through Node.js. Pass either `inline_code` (runs via `node -e`) or `script_path` (runs a file in the workspace). Optional `args` forwards positional arguments to the script."
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
                    "description": "Optional override for the default 300s timeout. Capped at 1800s."
                }
            }
        })
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

        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(NODE_TIMEOUT_SECS)
            .min(1800);

        if inline_code.is_some() == script_path.is_some() {
            return Ok(ToolResult::error(
                "node_exec requires exactly one of `inline_code` or `script_path`",
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
            let resolved_script = match resolve_script_path(&self.security.workspace_dir, path) {
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

        let mut cmd = match self
            .runtime
            .build_shell_command(&command, &self.security.workspace_dir)
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
            Ok(Err(e)) => Ok(ToolResult::error(format!("Failed to execute node: {e}"))),
            Err(_) => Ok(ToolResult::error(format!(
                "node_exec timed out after {timeout_secs}s and was killed"
            ))),
        }
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

    #[test]
    fn shell_quote_wraps_plain_strings() {
        assert_eq!(shell_quote("node"), "'node'");
        assert_eq!(shell_quote("/opt/bin/node"), "'/opt/bin/node'");
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
        assert!(resolve_script_path(ws, "/etc/passwd").is_err());
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
}
