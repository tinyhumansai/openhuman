use crate::openhuman::agent::host_runtime::RuntimeAdapter;
use crate::openhuman::javascript::NodeBootstrap;
use crate::openhuman::security::{AuditLogger, CommandExecutionLog, GateDecision, SecurityPolicy};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Maximum shell command execution time before kill.
const SHELL_TIMEOUT_SECS: u64 = 60;
/// Maximum output size in bytes (1MB).
const MAX_OUTPUT_BYTES: usize = 1_048_576;
/// Environment variables safe to pass to shell commands.
/// Only functional variables are included — never API keys or secrets.
const SAFE_ENV_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "TERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "USER",
    "SHELL",
    "TMPDIR",
    // Windows process creation and child command lookup need these after env_clear().
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

/// Shell command execution tool with sandboxing
pub struct ShellTool {
    security: Arc<SecurityPolicy>,
    runtime: Arc<dyn RuntimeAdapter>,
    audit: Arc<AuditLogger>,
    /// Optional managed Node.js bootstrap. When provided **and** a prior
    /// `NodeBootstrap::resolve()` has already succeeded, every shell invocation
    /// transparently prepends the managed `bin/` dir to `PATH` — so skills
    /// shelling out to `node`/`npm`/`npx`/`corepack` resolve to the managed
    /// toolchain. Non-blocking: never triggers a download for unrelated
    /// commands (we use `try_cached()`).
    node_bootstrap: Option<Arc<NodeBootstrap>>,
}

impl ShellTool {
    pub fn new(
        security: Arc<SecurityPolicy>,
        runtime: Arc<dyn RuntimeAdapter>,
        audit: Arc<AuditLogger>,
    ) -> Self {
        Self {
            security,
            runtime,
            audit,
            node_bootstrap: None,
        }
    }

    /// Same as `new` but attaches a managed Node.js bootstrap for transparent
    /// `PATH` injection. The bootstrap is consulted via `try_cached()` on each
    /// invocation, so calling a non-node shell command never forces a download.
    pub fn with_node_bootstrap(
        security: Arc<SecurityPolicy>,
        runtime: Arc<dyn RuntimeAdapter>,
        audit: Arc<AuditLogger>,
        bootstrap: Arc<NodeBootstrap>,
    ) -> Self {
        Self {
            security,
            runtime,
            audit,
            node_bootstrap: Some(bootstrap),
        }
    }

    /// Emit a single `CommandExecution` audit event. A write failure is logged
    /// as a structured warning but not propagated — audit must never block or
    /// fail a tool call, yet a silently broken audit trail must not go
    /// unnoticed.
    fn emit_audit(
        &self,
        command: &str,
        approved: bool,
        allowed: bool,
        success: bool,
        duration_ms: u64,
    ) {
        if let Err(error) = self.audit.log_command_event(CommandExecutionLog {
            channel: "tool:shell",
            command,
            risk_level: "unknown",
            approved,
            allowed,
            success,
            duration_ms,
        }) {
            tracing::warn!(
                error = %error,
                channel = "tool:shell",
                "[shell] failed to persist command execution audit event"
            );
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
                "category": {
                    "type": "string",
                    "enum": ["read", "write", "network", "install", "destructive"],
                    "description": "Optional self-declared risk category for this command. Advisory and ESCALATE-ONLY: it can raise the approval requirement (e.g. flag a destructive command) but never lowers what the runtime determines."
                }
            },
            "required": ["command"]
        })
    }

    /// Cap shell output at ~30k chars before threading into history.
    /// Verbose commands (`find /`, dependency installs, log dumps)
    /// can otherwise blow past 100k chars in one call. The agent
    /// rarely needs the full firehose — a head/tail/grep follow-up is
    /// the right move when it does.
    fn max_result_size_chars(&self) -> Option<usize> {
        Some(30_000)
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    /// Whether this shell call must be approved by the human before it runs.
    /// True for any command the current tier prompts on (Write / Network /
    /// Destructive in ask-before-edit; Network / Destructive in Full). The
    /// harness routes these through the `ApprovalGate`; the read-only `Block`
    /// and the structural guard are enforced in `run_with_security`.
    fn external_effect_with_args(&self, args: &serde_json::Value) -> bool {
        let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
        let mut class = self.security.classify_command(command);
        // Escalate-only LLM hint: max() so a self-declared category can raise
        // the requirement (e.g. Write -> Destructive) but never lower it.
        if let Some(declared) = args
            .get("category")
            .and_then(|v| v.as_str())
            .and_then(SecurityPolicy::parse_declared_class)
        {
            class = class.max(declared);
        }
        self.security.gate_decision(class) == GateDecision::Prompt
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'command' parameter"))?;

        let start = Instant::now();
        let (allowed, result) = self.run_with_security(command).await;
        let duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        // `allowed` = passed the in-tool security checks. `approved` = the command
        // is Prompt-class (required human approval) and thus went through the
        // harness ApprovalGate to reach here — distinct from `allowed`. Reads and
        // Full-mode writes run without a prompt, so they audit as approved=false
        // rather than over-claiming a human approval that never happened. (The
        // gate's exact yes/no isn't threaded into tools; this is the accurate
        // "required approval" proxy.)
        let approved = self.external_effect_with_args(&args);
        // emit_audit signature is (command, approved, allowed, …) — keep that order.
        self.emit_audit(command, approved, allowed, !result.is_error, duration_ms);
        Ok(result)
    }
}

impl ShellTool {
    /// Run the command through the security policy and runtime. Returns
    /// `(allowed, result)` where `allowed=false` means the policy or rate
    /// limiter blocked execution before the command was launched.
    ///
    /// Exposed as `pub(crate)` so workflow phase scripts can reuse the
    /// same gated execution path as the `shell` tool — all security
    /// checks (rate limits, path guards, approval gate routing) apply
    /// identically to workflow-triggered commands.
    pub(crate) async fn run_with_security(&self, command: &str) -> (bool, ToolResult) {
        // Read-only `Block` + the Option-2 structural guard. Approval for
        // Write / Network / Destructive already happened at the harness
        // `ApprovalGate` (see `external_effect_with_args`) before `execute()`
        // ran; this enforces what must still hold afterwards.
        if let Err(reason) = self.security.check_gated_command(command) {
            return (false, ToolResult::error(reason));
        }

        if self.security.is_rate_limited() {
            return (
                false,
                ToolResult::error("Rate limit exceeded: too many actions in the last hour"),
            );
        }

        if !self.security.record_action() {
            return (
                false,
                ToolResult::error("Rate limit exceeded: action budget exhausted"),
            );
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
                return (
                    true,
                    ToolResult::error(format!("Failed to build runtime command: {e}")),
                );
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

        let tool_result = match result {
            Ok(Ok(output)) => {
                let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();

                // Truncate output to prevent OOM
                if stdout.len() > MAX_OUTPUT_BYTES {
                    stdout.truncate(crate::openhuman::util::floor_char_boundary(
                        &stdout,
                        MAX_OUTPUT_BYTES,
                    ));
                    stdout.push_str("\n... [output truncated at 1MB]");
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
                        ToolResult::success(stdout)
                    } else {
                        // Successful exit but stderr present — attach stderr as output suffix
                        ToolResult::success(format!("{stdout}\n[stderr]\n{stderr}"))
                    }
                } else {
                    let err_msg = if stderr.is_empty() { stdout } else { stderr };
                    ToolResult::error(err_msg)
                }
            }
            Ok(Err(e)) => ToolResult::error(format!("Failed to execute command: {e}")),
            Err(_) => ToolResult::error(format!(
                "Command timed out after {SHELL_TIMEOUT_SECS}s and was killed"
            )),
        };
        (true, tool_result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::host_runtime::{NativeRuntime, RuntimeAdapter};
    use crate::openhuman::security::{AutonomyLevel, CommandClass, SecurityPolicy};

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

    fn test_audit() -> Arc<AuditLogger> {
        AuditLogger::disabled()
    }

    fn audit_with_tempdir() -> (Arc<AuditLogger>, tempfile::TempDir) {
        use crate::openhuman::config::AuditConfig;
        let tmp = tempfile::tempdir().expect("create tempdir");
        let logger = AuditLogger::new(
            AuditConfig {
                enabled: true,
                log_path: "audit.log".into(),
                max_size_mb: 10,
            },
            tmp.path().to_path_buf(),
        )
        .expect("create audit logger");
        (Arc::new(logger), tmp)
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn shell_emits_audit_line_on_success() {
        use crate::openhuman::security::AuditEvent;
        let (audit, tmp) = audit_with_tempdir();
        let tool = ShellTool::new(
            test_security(AutonomyLevel::Supervised),
            test_runtime(),
            audit,
        );
        let _ = tool
            .execute(json!({"command": "echo hello"}))
            .await
            .unwrap();
        let log = std::fs::read_to_string(tmp.path().join("audit.log"))
            .expect("audit log file should exist");
        assert!(!log.is_empty(), "audit log should not be empty");
        let parsed: AuditEvent = serde_json::from_str(log.trim()).expect("audit event JSON parses");
        let action = parsed.action.expect("action present");
        assert_eq!(action.command, Some("echo hello".to_string()));
        assert!(action.allowed, "allowed command should set allowed=true");
        let result = parsed.result.expect("result present");
        assert!(result.success, "echo hello should succeed");
        let actor = parsed.actor.expect("actor present");
        assert_eq!(actor.channel, "tool:shell");
    }

    #[tokio::test]
    async fn shell_emits_audit_line_on_denial() {
        use crate::openhuman::security::AuditEvent;
        let (audit, tmp) = audit_with_tempdir();
        let tool = ShellTool::new(
            test_security(AutonomyLevel::ReadOnly),
            test_runtime(),
            audit,
        );
        // A write command in read-only mode is denied before execution.
        let _ = tool
            .execute(json!({"command": "touch denied_file"}))
            .await
            .unwrap();
        let log = std::fs::read_to_string(tmp.path().join("audit.log"))
            .expect("audit log file should exist");
        let parsed: AuditEvent = serde_json::from_str(log.trim()).expect("audit event JSON parses");
        let action = parsed.action.expect("action present");
        assert!(
            !action.allowed,
            "denied command should set allowed=false on the audit event"
        );
    }

    #[test]
    fn shell_tool_name() {
        let tool = ShellTool::new(
            test_security(AutonomyLevel::Supervised),
            test_runtime(),
            test_audit(),
        );
        assert_eq!(tool.name(), "shell");
    }

    #[test]
    fn shell_tool_description() {
        let tool = ShellTool::new(
            test_security(AutonomyLevel::Supervised),
            test_runtime(),
            test_audit(),
        );
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn shell_tool_schema_has_command() {
        let tool = ShellTool::new(
            test_security(AutonomyLevel::Supervised),
            test_runtime(),
            test_audit(),
        );
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["command"].is_object());
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("command")));
        // The self-asserted `approved` param was removed — approval now happens
        // at the harness ApprovalGate, not via a model-set flag.
        assert!(schema["properties"]["approved"].is_null());
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn shell_executes_allowed_command() {
        let tool = ShellTool::new(
            test_security(AutonomyLevel::Supervised),
            test_runtime(),
            test_audit(),
        );
        let result = tool
            .execute(json!({"command": "echo hello"}))
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.output());
        assert!(result.output().trim().contains("hello"));
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn shell_destructive_command_is_gated_not_run_inline() {
        // `rm -rf /` is Destructive → it must route through the human approval
        // gate (external_effect), never auto-run. Assert the classification
        // here rather than executing it.
        let security = test_security(AutonomyLevel::Supervised);
        let tool = ShellTool::new(security.clone(), test_runtime(), test_audit());
        assert_eq!(
            security.classify_command("rm -rf /"),
            CommandClass::Destructive
        );
        assert!(tool.external_effect_with_args(&json!({"command": "rm -rf /"})));
    }

    #[tokio::test]
    async fn shell_readonly_allows_reads_blocks_writes() {
        let security = test_security(AutonomyLevel::ReadOnly);
        // Read commands are permitted in read-only mode…
        assert_eq!(
            security.gate_decision(security.classify_command("ls")),
            GateDecision::Allow
        );
        // …but a write command is blocked before execution.
        let tool = ShellTool::new(security, test_runtime(), test_audit());
        let blocked = tool
            .execute(json!({"command": "touch ro_test_file"}))
            .await
            .unwrap();
        assert!(blocked.is_error);
        assert!(blocked.output().contains("read-only"));
    }

    #[tokio::test]
    async fn shell_missing_command_param() {
        let tool = ShellTool::new(
            test_security(AutonomyLevel::Supervised),
            test_runtime(),
            test_audit(),
        );
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("command"));
    }

    #[tokio::test]
    async fn shell_wrong_type_param() {
        let tool = ShellTool::new(
            test_security(AutonomyLevel::Supervised),
            test_runtime(),
            test_audit(),
        );
        let result = tool.execute(json!({"command": 123})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shell_captures_exit_code() {
        let tool = ShellTool::new(
            test_security(AutonomyLevel::Supervised),
            test_runtime(),
            test_audit(),
        );
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
            allowed_commands: vec!["echo".into(), "mkdir".into()],
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

    #[cfg(not(windows))]
    #[tokio::test(flavor = "current_thread")]
    async fn shell_does_not_leak_api_key() {
        let _g1 = EnvGuard::set("API_KEY", "sk-test-secret-12345");

        let tool = ShellTool::new(test_security_with_env_cmd(), test_runtime(), test_audit());
        let result = tool
            .execute(json!({"command": "echo $API_KEY"}))
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.output());
        assert!(
            !result.output().contains("sk-test-secret-12345"),
            "API_KEY leaked to shell command output"
        );
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn shell_preserves_path_and_home() {
        let tool = ShellTool::new(test_security_with_env_cmd(), test_runtime(), test_audit());

        let result = tool
            .execute(json!({"command": "echo $HOME"}))
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.output());
        assert!(
            !result.output().trim().is_empty(),
            "HOME should be available in shell"
        );

        let result = tool
            .execute(json!({"command": "echo $PATH"}))
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.output());
        assert!(
            !result.output().trim().is_empty(),
            "PATH should be available in shell"
        );
    }

    #[tokio::test]
    async fn shell_writes_are_gated_in_supervised_run_in_full() {
        // A write command routes through the approval gate in ask-before-edit
        // (no self-asserted `approved` flag any more)…
        let supervised = test_security(AutonomyLevel::Supervised);
        let tool = ShellTool::new(supervised.clone(), test_runtime(), test_audit());
        assert_eq!(supervised.classify_command("touch f"), CommandClass::Write);
        assert!(tool.external_effect_with_args(&json!({"command": "touch f"})));

        // …and runs without prompting in Full.
        let full = test_security(AutonomyLevel::Full);
        let full_tool = ShellTool::new(full, test_runtime(), test_audit());
        assert!(!full_tool.external_effect_with_args(&json!({"command": "touch f"})));
    }

    #[tokio::test]
    async fn shell_llm_category_escalates_but_never_lowers() {
        // In Full a Write runs silently…
        let full = test_security(AutonomyLevel::Full);
        let tool = ShellTool::new(full, test_runtime(), test_audit());
        assert!(!tool.external_effect_with_args(&json!({"command": "touch f"})));
        // …but a self-declared `destructive` escalates it to a prompt.
        assert!(tool
            .external_effect_with_args(&json!({"command": "touch f", "category": "destructive"})));
        // The hint can never LOWER: declaring a destructive command "read"
        // still prompts (in any acting tier).
        let supervised = test_security(AutonomyLevel::Supervised);
        let stool = ShellTool::new(supervised, test_runtime(), test_audit());
        assert!(
            stool.external_effect_with_args(&json!({"command": "sudo reboot", "category": "read"}))
        );
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

    #[test]
    fn shell_safe_env_vars_include_windows_process_essentials() {
        for var in ["SystemRoot", "COMSPEC", "PATHEXT", "TEMP", "USERPROFILE"] {
            assert!(
                SAFE_ENV_VARS.contains(&var),
                "{var} must be forwarded for Windows child processes"
            );
        }
    }

    #[tokio::test]
    async fn shell_blocks_rate_limited() {
        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            max_actions_per_hour: 0,
            workspace_dir: std::env::temp_dir(),
            ..SecurityPolicy::default()
        });
        let tool = ShellTool::new(security, test_runtime(), test_audit());
        let result = tool.execute(json!({"command": "echo test"})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("Rate limit"));
    }
}
