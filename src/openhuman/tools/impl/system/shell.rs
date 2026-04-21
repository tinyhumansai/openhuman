use crate::openhuman::agent::host_runtime::RuntimeAdapter;
use crate::openhuman::node_runtime::NodeBootstrap;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// Maximum shell command execution time before kill.
const SHELL_TIMEOUT_SECS: u64 = 60;
/// Maximum output size in bytes (1MB).
const MAX_OUTPUT_BYTES: usize = 1_048_576;
/// Environment variables safe to pass to shell commands.
/// Only functional variables are included — never API keys or secrets.
const SAFE_ENV_VARS: &[&str] = &[
    "PATH", "HOME", "TERM", "LANG", "LC_ALL", "LC_CTYPE", "USER", "SHELL", "TMPDIR",
];

/// Shell command execution tool with sandboxing
pub struct ShellTool {
    security: Arc<SecurityPolicy>,
    runtime: Arc<dyn RuntimeAdapter>,
    /// Optional managed Node.js bootstrap. When provided **and** a prior
    /// `NodeBootstrap::resolve()` has already succeeded, every shell invocation
    /// transparently prepends the managed `bin/` dir to `PATH` — so skills
    /// shelling out to `node`/`npm`/`npx`/`corepack` resolve to the managed
    /// toolchain. Non-blocking: never triggers a download for unrelated
    /// commands (we use `try_cached()`).
    node_bootstrap: Option<Arc<NodeBootstrap>>,
}

impl ShellTool {
    pub fn new(security: Arc<SecurityPolicy>, runtime: Arc<dyn RuntimeAdapter>) -> Self {
        Self {
            security,
            runtime,
            node_bootstrap: None,
        }
    }

    /// Same as `new` but attaches a managed Node.js bootstrap for transparent
    /// `PATH` injection. The bootstrap is consulted via `try_cached()` on each
    /// invocation, so calling a non-node shell command never forces a download.
    pub fn with_node_bootstrap(
        security: Arc<SecurityPolicy>,
        runtime: Arc<dyn RuntimeAdapter>,
        bootstrap: Arc<NodeBootstrap>,
    ) -> Self {
        Self {
            security,
            runtime,
            node_bootstrap: Some(bootstrap),
        }
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "Execute a shell command in the workspace directory"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "approved": {
                    "type": "boolean",
                    "description": "Set true to explicitly approve medium/high-risk commands in supervised mode",
                    "default": false
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'command' parameter"))?;
        let approved = args
            .get("approved")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if self.security.is_rate_limited() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: too many actions in the last hour",
            ));
        }

        match self.security.validate_command_execution(command, approved) {
            Ok(_) => {}
            Err(reason) => {
                return Ok(ToolResult::error(reason));
            }
        }

        if !self.security.record_action() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: action budget exhausted",
            ));
        }

        // Execute with timeout to prevent hanging commands.
        // Clear the environment to prevent leaking API keys and other secrets
        // (CWE-200), then re-add only safe, functional variables.
        let mut cmd = match self
            .runtime
            .build_shell_command(command, &self.security.workspace_dir)
        {
            Ok(cmd) => cmd,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Failed to build runtime command: {e}"
                )));
            }
        };
        cmd.env_clear();

        for var in SAFE_ENV_VARS {
            if let Ok(val) = std::env::var(var) {
                cmd.env(var, val);
            }
        }

        // If a managed Node.js install has already been resolved, transparently
        // prepend its bin dir to PATH so this shell sees the managed toolchain.
        // `try_cached()` never blocks and never triggers a download — unrelated
        // commands (e.g. `ls`) stay fast and byte-identical to before.
        if let Some(bootstrap) = self.node_bootstrap.as_ref() {
            if let Some(resolved) = bootstrap.try_cached() {
                let host_path = std::env::var("PATH").unwrap_or_default();
                let sep = if cfg!(windows) { ";" } else { ":" };
                let prepended = if host_path.is_empty() {
                    resolved.bin_dir.to_string_lossy().into_owned()
                } else {
                    format!("{}{}{}", resolved.bin_dir.display(), sep, host_path)
                };
                tracing::debug!(
                    bin_dir = %resolved.bin_dir.display(),
                    version = %resolved.version,
                    "[shell] prepending managed node bin to PATH"
                );
                cmd.env("PATH", prepended);
            }
        }

        let result =
            tokio::time::timeout(Duration::from_secs(SHELL_TIMEOUT_SECS), cmd.output()).await;

        match result {
            Ok(Ok(output)) => {
                let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();

                // Truncate output to prevent OOM
                if stdout.len() > MAX_OUTPUT_BYTES {
                    stdout.truncate(stdout.floor_char_boundary(MAX_OUTPUT_BYTES));
                    stdout.push_str("\n... [output truncated at 1MB]");
                }
                if stderr.len() > MAX_OUTPUT_BYTES {
                    stderr.truncate(stderr.floor_char_boundary(MAX_OUTPUT_BYTES));
                    stderr.push_str("\n... [stderr truncated at 1MB]");
                }

                if output.status.success() {
                    if stderr.is_empty() {
                        Ok(ToolResult::success(stdout))
                    } else {
                        // Successful exit but stderr present — attach stderr as output suffix
                        Ok(ToolResult::success(format!("{stdout}\n[stderr]\n{stderr}")))
                    }
                } else {
                    let err_msg = if stderr.is_empty() { stdout } else { stderr };
                    Ok(ToolResult::error(err_msg))
                }
            }
            Ok(Err(e)) => Ok(ToolResult::error(format!("Failed to execute command: {e}"))),
            Err(_) => Ok(ToolResult::error(format!(
                "Command timed out after {SHELL_TIMEOUT_SECS}s and was killed"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::host_runtime::{NativeRuntime, RuntimeAdapter};
    use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};

    fn test_security(autonomy: AutonomyLevel) -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy,
            workspace_dir: std::env::temp_dir(),
            ..SecurityPolicy::default()
        })
    }

    fn test_runtime() -> Arc<dyn RuntimeAdapter> {
        Arc::new(NativeRuntime::new())
    }

    #[test]
    fn shell_tool_name() {
        let tool = ShellTool::new(test_security(AutonomyLevel::Supervised), test_runtime());
        assert_eq!(tool.name(), "shell");
    }

    #[test]
    fn shell_tool_description() {
        let tool = ShellTool::new(test_security(AutonomyLevel::Supervised), test_runtime());
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn shell_tool_schema_has_command() {
        let tool = ShellTool::new(test_security(AutonomyLevel::Supervised), test_runtime());
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["command"].is_object());
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("command")));
        assert!(schema["properties"]["approved"].is_object());
    }

    #[tokio::test]
    async fn shell_executes_allowed_command() {
        let tool = ShellTool::new(test_security(AutonomyLevel::Supervised), test_runtime());
        let result = tool
            .execute(json!({"command": "echo hello"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output().trim().contains("hello"));
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn shell_blocks_disallowed_command() {
        let tool = ShellTool::new(test_security(AutonomyLevel::Supervised), test_runtime());
        let result = tool.execute(json!({"command": "rm -rf /"})).await.unwrap();
        assert!(result.is_error);
        let error = result.output();
        assert!(error.contains("not allowed") || error.contains("high-risk"));
    }

    #[tokio::test]
    async fn shell_blocks_readonly() {
        let tool = ShellTool::new(test_security(AutonomyLevel::ReadOnly), test_runtime());
        let result = tool.execute(json!({"command": "ls"})).await.unwrap();
        assert!(result.is_error);
        assert!(&result.output().contains("not allowed"));
    }

    #[tokio::test]
    async fn shell_missing_command_param() {
        let tool = ShellTool::new(test_security(AutonomyLevel::Supervised), test_runtime());
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("command"));
    }

    #[tokio::test]
    async fn shell_wrong_type_param() {
        let tool = ShellTool::new(test_security(AutonomyLevel::Supervised), test_runtime());
        let result = tool.execute(json!({"command": 123})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shell_captures_exit_code() {
        let tool = ShellTool::new(test_security(AutonomyLevel::Supervised), test_runtime());
        let result = tool
            .execute(json!({"command": "ls /nonexistent_dir_xyz"}))
            .await
            .unwrap();
        assert!(result.is_error);
    }

    fn test_security_with_env_cmd() -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: std::env::temp_dir(),
            allowed_commands: vec!["env".into(), "echo".into()],
            ..SecurityPolicy::default()
        })
    }

    /// RAII guard that restores an environment variable to its original state on drop,
    /// ensuring cleanup even if the test panics.
    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(val) => std::env::set_var(self.key, val),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shell_does_not_leak_api_key() {
        let _g1 = EnvGuard::set("API_KEY", "sk-test-secret-12345");
        let _g2 = EnvGuard::set("OPENHUMAN_API_KEY", "sk-test-secret-67890");

        let tool = ShellTool::new(test_security_with_env_cmd(), test_runtime());
        let result = tool.execute(json!({"command": "env"})).await.unwrap();
        assert!(!result.is_error);
        assert!(
            !result.output().contains("sk-test-secret-12345"),
            "API_KEY leaked to shell command output"
        );
        assert!(
            !result.output().contains("sk-test-secret-67890"),
            "OPENHUMAN_API_KEY leaked to shell command output"
        );
    }

    #[tokio::test]
    async fn shell_preserves_path_and_home() {
        let tool = ShellTool::new(test_security_with_env_cmd(), test_runtime());

        let result = tool
            .execute(json!({"command": "echo $HOME"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(
            !result.output().trim().is_empty(),
            "HOME should be available in shell"
        );

        let result = tool
            .execute(json!({"command": "echo $PATH"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(
            !result.output().trim().is_empty(),
            "PATH should be available in shell"
        );
    }

    #[tokio::test]
    async fn shell_requires_approval_for_medium_risk_command() {
        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            allowed_commands: vec!["touch".into()],
            workspace_dir: std::env::temp_dir(),
            ..SecurityPolicy::default()
        });

        let tool = ShellTool::new(security.clone(), test_runtime());
        let denied = tool
            .execute(json!({"command": "touch openhuman_shell_approval_test"}))
            .await
            .unwrap();
        assert!(denied.is_error);
        assert!(denied.output().contains("explicit approval"));

        let allowed = tool
            .execute(json!({
                "command": "touch openhuman_shell_approval_test",
                "approved": true
            }))
            .await
            .unwrap();
        assert!(!allowed.is_error);

        let _ = tokio::fs::remove_file(std::env::temp_dir().join("openhuman_shell_approval_test"))
            .await;
    }

    // ── §5.2 Shell timeout enforcement tests ─────────────────

    #[test]
    fn shell_timeout_constant_is_reasonable() {
        assert_eq!(SHELL_TIMEOUT_SECS, 60, "shell timeout must be 60 seconds");
    }

    #[test]
    fn shell_output_limit_is_1mb() {
        assert_eq!(
            MAX_OUTPUT_BYTES, 1_048_576,
            "max output must be 1 MB to prevent OOM"
        );
    }

    // ── §5.3 Non-UTF8 binary output tests ────────────────────

    #[test]
    fn shell_safe_env_vars_excludes_secrets() {
        for var in SAFE_ENV_VARS {
            let lower = var.to_lowercase();
            assert!(
                !lower.contains("key") && !lower.contains("secret") && !lower.contains("token"),
                "SAFE_ENV_VARS must not include sensitive variable: {var}"
            );
        }
    }

    #[test]
    fn shell_safe_env_vars_includes_essentials() {
        assert!(
            SAFE_ENV_VARS.contains(&"PATH"),
            "PATH must be in safe env vars"
        );
        assert!(
            SAFE_ENV_VARS.contains(&"HOME"),
            "HOME must be in safe env vars"
        );
        assert!(
            SAFE_ENV_VARS.contains(&"TERM"),
            "TERM must be in safe env vars"
        );
    }

    #[tokio::test]
    async fn shell_blocks_rate_limited() {
        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            max_actions_per_hour: 0,
            workspace_dir: std::env::temp_dir(),
            ..SecurityPolicy::default()
        });
        let tool = ShellTool::new(security, test_runtime());
        let result = tool.execute(json!({"command": "echo test"})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("Rate limit"));
    }
}
