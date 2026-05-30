//! Tool: install_tool — install an OS / language package, heavily gated.
//!
//! This is the highest-blast-radius tool: it changes the user's machine. It is
//! therefore gated on `SecurityPolicy.allow_tool_install` (intended to be set
//! only in Full access mode), runs the package manager via an explicit argv
//! (never a shell string, so the package name can't inject commands), validates
//! the package name against a strict charset, and records an audit log line.
//!
//! It deliberately does NOT auto-prepend `sudo`: system managers that need root
//! (apt/dnf/pacman) will surface a permissions error, which the agent should
//! relay rather than silently escalate. User-scoped managers (brew, pipx,
//! `npm -g`, `cargo install`) work without elevation.

use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command as TokioCommand;
use tokio::time::timeout;

use super::detect_tools::find_on_path;

/// Hard cap on an install before it is killed.
const INSTALL_TIMEOUT_SECS: u64 = 300;
/// Per-stream cap on captured installer output (1 MiB) — verbose package
/// managers can emit very large stdout/stderr and spike memory.
const MAX_OUTPUT_BYTES: usize = 1_048_576;

pub struct InstallToolTool {
    security: Arc<SecurityPolicy>,
}

impl InstallToolTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }
}

/// Map a manager id to its binary + the argv prefix that precedes the package.
fn manager_argv(manager: &str) -> Option<(&'static str, Vec<&'static str>)> {
    match manager {
        "apt" | "apt-get" => Some(("apt-get", vec!["install", "-y"])),
        "dnf" => Some(("dnf", vec!["install", "-y"])),
        "yum" => Some(("yum", vec!["install", "-y"])),
        "pacman" => Some(("pacman", vec!["-S", "--noconfirm"])),
        "apk" => Some(("apk", vec!["add"])),
        "brew" => Some(("brew", vec!["install"])),
        "winget" => Some(("winget", vec!["install", "-e", "--id"])),
        "pipx" => Some(("pipx", vec!["install"])),
        "npm" => Some(("npm", vec!["install", "-g"])),
        "cargo" => Some(("cargo", vec!["install"])),
        _ => None,
    }
}

/// Best-effort detection of the host's system package manager.
fn detect_system_manager() -> Option<&'static str> {
    for m in ["apt-get", "dnf", "yum", "pacman", "apk", "brew", "winget"] {
        if find_on_path(m).is_some() {
            return Some(m);
        }
    }
    None
}

/// A package name is accepted only if it contains exclusively characters that
/// appear in real package identifiers — letters, digits and `._+-@/:`. This
/// blocks shell metacharacters even though we never build a shell string.
fn is_valid_package_name(pkg: &str) -> bool {
    !pkg.is_empty()
        && pkg.len() <= 200
        && pkg.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-' | '@' | '/' | ':')
        })
}

#[async_trait]
impl Tool for InstallToolTool {
    fn name(&self) -> &str {
        "install_tool"
    }

    fn description(&self) -> &str {
        "Install an OS or language package on the host (apt/dnf/pacman/apk/brew/winget/pipx/\
         npm -g/cargo). HIGH IMPACT: only available when the user has enabled tool installation \
         (Full access mode). Prefer detect_tools first; only install what's actually missing."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "package": {
                    "type": "string",
                    "description": "Package name to install (e.g. 'ripgrep', 'jq', '@scope/cli')."
                },
                "manager": {
                    "type": "string",
                    "enum": ["apt", "apt-get", "dnf", "yum", "pacman", "apk", "brew", "winget", "pipx", "npm", "cargo"],
                    "description": "Package manager to use. If omitted, the host's system package manager is auto-detected."
                }
            },
            "required": ["package"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Dangerous
    }

    fn external_effect(&self) -> bool {
        true
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        // Gate 0: installs mutate the host (not the workspace), so they must
        // never run without an explicit human Approve. The ApprovalGate only
        // parks for an interactive turn (`APPROVAL_CHAT_CONTEXT` present, set on
        // the web-chat path); background / triage / cron turns bypass the gate
        // entirely. Fail closed there — a doomed retry is short-circuited by the
        // `[policy-denied]` marker.
        if crate::openhuman::approval::APPROVAL_CHAT_CONTEXT
            .try_with(|_| ())
            .is_err()
        {
            return Ok(ToolResult::error(
                "[policy-denied] install_tool requires interactive approval and is not available \
                 in autonomous (background) turns.",
            ));
        }
        // Gate 1: feature must be explicitly enabled (Full access mode).
        if !self.security.allow_tool_install {
            return Ok(ToolResult::error(
                "[policy-denied] OS package installation is disabled. Enable it in the agent access settings \
                 (Full access mode / allow_tool_install) before installing tools.",
            ));
        }
        // Gate 2: read-only autonomy never installs.
        if !self.security.can_act() {
            return Ok(ToolResult::error(
                "[policy-blocked] Action blocked: autonomy is read-only.",
            ));
        }
        // Gate 3: rate limit.
        if self.security.is_rate_limited() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: too many actions in the last hour",
            ));
        }

        let package = match args.get("package").and_then(|v| v.as_str()) {
            Some(p) => p.trim(),
            None => return Ok(ToolResult::error("Missing 'package' parameter")),
        };
        if !is_valid_package_name(package) {
            return Ok(ToolResult::error(format!(
                "Invalid package name '{package}': only letters, digits and ._+-@/: are allowed"
            )));
        }

        let manager =
            match args.get("manager").and_then(|v| v.as_str()) {
                Some(m) => m.to_string(),
                None => match detect_system_manager() {
                    Some(m) => m.to_string(),
                    None => return Ok(ToolResult::error(
                        "No supported package manager detected on PATH. Pass 'manager' explicitly \
                         (e.g. brew, pipx, npm, cargo).",
                    )),
                },
            };

        let Some((program, prefix)) = manager_argv(&manager) else {
            return Ok(ToolResult::error(format!(
                "Unsupported package manager '{manager}'"
            )));
        };
        if find_on_path(program).is_none() {
            return Ok(ToolResult::error(format!(
                "Package manager '{program}' is not installed on PATH"
            )));
        }

        // Spend an action against the rate budget.
        if !self.security.record_action() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: action budget exhausted",
            ));
        }

        tracing::warn!(
            channel = "tool:install_tool",
            manager = %program,
            package = %package,
            "[install_tool] installing OS package (gated by allow_tool_install)"
        );

        let mut cmd = TokioCommand::new(program);
        cmd.args(&prefix).arg(package);
        cmd.current_dir(&self.security.workspace_dir);

        let output = match timeout(Duration::from_secs(INSTALL_TIMEOUT_SECS), cmd.output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                return Ok(ToolResult::error(format!(
                    "Failed to launch {program}: {e}"
                )))
            }
            Err(_) => {
                return Ok(ToolResult::error(format!(
                    "Install timed out after {INSTALL_TIMEOUT_SECS}s and was killed"
                )))
            }
        };

        let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if stdout.len() > MAX_OUTPUT_BYTES {
            stdout.truncate(crate::openhuman::util::floor_char_boundary(
                &stdout,
                MAX_OUTPUT_BYTES,
            ));
            stdout.push_str("\n... [stdout truncated at 1 MiB]");
        }
        if stderr.len() > MAX_OUTPUT_BYTES {
            stderr.truncate(crate::openhuman::util::floor_char_boundary(
                &stderr,
                MAX_OUTPUT_BYTES,
            ));
            stderr.push_str("\n... [stderr truncated at 1 MiB]");
        }
        if output.status.success() {
            Ok(ToolResult::success(format!(
                "Installed '{package}' via {program}.\n{stdout}"
            )))
        } else {
            Ok(ToolResult::error(format!(
                "{program} failed to install '{package}' (exit {:?}).\n{}",
                output.status.code(),
                if stderr.is_empty() { stdout } else { stderr }
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::approval::{ApprovalChatContext, APPROVAL_CHAT_CONTEXT};
    use crate::openhuman::security::AutonomyLevel;

    fn policy(allow_install: bool, autonomy: AutonomyLevel) -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy,
            allow_tool_install: allow_install,
            workspace_dir: std::env::temp_dir(),
            ..SecurityPolicy::default()
        })
    }

    // install_tool refuses outside an interactive (approval) turn — Gate 0 — so
    // tests that exercise the *other* gates run inside a chat context.
    fn chat_ctx() -> ApprovalChatContext {
        ApprovalChatContext {
            thread_id: "t-test".into(),
            client_id: "c-test".into(),
        }
    }

    #[tokio::test]
    async fn blocked_when_install_disabled() {
        let tool = InstallToolTool::new(policy(false, AutonomyLevel::Full));
        let result = APPROVAL_CHAT_CONTEXT
            .scope(chat_ctx(), tool.execute(json!({ "package": "jq" })))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("disabled"), "{}", result.output());
    }

    #[tokio::test]
    async fn blocked_when_readonly() {
        let tool = InstallToolTool::new(policy(true, AutonomyLevel::ReadOnly));
        let result = APPROVAL_CHAT_CONTEXT
            .scope(chat_ctx(), tool.execute(json!({ "package": "jq" })))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("read-only"), "{}", result.output());
    }

    #[tokio::test]
    async fn rejects_injection_in_package_name() {
        let tool = InstallToolTool::new(policy(true, AutonomyLevel::Full));
        let result = APPROVAL_CHAT_CONTEXT
            .scope(
                chat_ctx(),
                tool.execute(json!({ "package": "jq; rm -rf /" })),
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(
            result.output().contains("Invalid package name"),
            "{}",
            result.output()
        );
    }

    #[tokio::test]
    async fn refuses_in_autonomous_turn_without_chat_context() {
        // No APPROVAL_CHAT_CONTEXT scope → background/autonomous turn → refused
        // by Gate 0 before any install logic runs.
        let tool = InstallToolTool::new(policy(true, AutonomyLevel::Full));
        let result = tool.execute(json!({ "package": "jq" })).await.unwrap();
        assert!(result.is_error);
        assert!(
            result.output().contains("interactive approval"),
            "{}",
            result.output()
        );
    }

    #[tokio::test]
    async fn rejects_unknown_manager() {
        let tool = InstallToolTool::new(policy(true, AutonomyLevel::Full));
        let result = tool
            .execute(json!({ "package": "jq", "manager": "notamanager" }))
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[test]
    fn package_name_validation() {
        assert!(is_valid_package_name("ripgrep"));
        assert!(is_valid_package_name("@scope/cli"));
        assert!(is_valid_package_name("python3.11"));
        assert!(!is_valid_package_name("jq; rm -rf /"));
        assert!(!is_valid_package_name("a b"));
        assert!(!is_valid_package_name(""));
    }
}
