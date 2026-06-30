use crate::openhuman::agent::host_runtime::RuntimeAdapter;
use crate::openhuman::javascript::NodeBootstrap;
use crate::openhuman::runtime_python::PythonBootstrap;
use crate::openhuman::security::{AuditLogger, CommandExecutionLog, GateDecision, SecurityPolicy};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult, ToolTimeout};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    /// Optional managed Python bootstrap. Unlike Node PATH injection, Python
    /// shell support is the primary execution surface for skills, so
    /// Python-looking commands resolve this lazily before spawn. That keeps
    /// `pip install foo` and `python3 -m foo` on one interpreter instead of
    /// mixing arbitrary host `pip` and `python3` binaries.
    python_bootstrap: Option<Arc<PythonBootstrap>>,
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
            python_bootstrap: None,
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
            python_bootstrap: None,
        }
    }

    /// Attach managed language runtimes used by shell-invoked skills. Node is
    /// injected only after a dedicated node/npm tool resolved it; Python is
    /// resolved lazily for python/pip commands because shell is currently the
    /// user-facing Python skill execution path.
    pub fn with_language_bootstraps(
        security: Arc<SecurityPolicy>,
        runtime: Arc<dyn RuntimeAdapter>,
        audit: Arc<AuditLogger>,
        node_bootstrap: Option<Arc<NodeBootstrap>>,
        python_bootstrap: Option<Arc<PythonBootstrap>>,
    ) -> Self {
        Self {
            security,
            runtime,
            audit,
            node_bootstrap,
            python_bootstrap,
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

    /// Resolve the working directory for this shell invocation.
    ///
    /// Returns the per-worker git-worktree override when one is installed via
    /// [`crate::openhuman::agent::harness::with_action_dir_override`] (an
    /// edit-capable worker running with `isolation = "worktree"`), otherwise
    /// the shared `self.security.action_dir`. Keeping `security.action_dir` as
    /// the fallback preserves the non-isolated behaviour exactly. See #3376.
    fn effective_action_dir(&self) -> std::path::PathBuf {
        crate::openhuman::agent::harness::current_action_dir_override()
            .unwrap_or_else(|| self.security.action_dir.clone())
    }

    /// The explicit wall-clock budget for this invocation, or `None` to run
    /// unbounded.
    ///
    /// Shell commands run scripts — builds, test suites, solvers — that
    /// legitimately take minutes, so there is **no** default timeout: a
    /// deadline applies only when the caller passes `timeout_secs` (issue
    /// #4023). A `0` explicitly disables it. Any positive value is clamped to
    /// `1..=3600`. See
    /// [`crate::openhuman::tool_timeout::explicit_call_timeout_duration`].
    fn explicit_timeout(&self, requested: Option<u64>) -> Option<Duration> {
        crate::openhuman::tool_timeout::explicit_call_timeout_duration(
            requested,
            crate::openhuman::tool_timeout::MAX_TIMEOUT_SECS,
        )
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "Execute a shell command. Use this to run code, manipulate files in the workspace, \
         or perform system actions on the user's machine — including launching applications \
         (e.g. `open -a Music` on macOS, `xdg-open music://` on Linux). Only the command's \
         stdout/stderr is captured and returned to you — a program that prints nothing \
         (e.g. a `python`/`node` script that computes silently or only writes a file) returns \
         an empty result, so make scripts print the output you need (e.g. `print(...)`), or \
         follow up by reading any file they wrote."
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
                },
                "timeout_secs": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 3600,
                    "description": "Optional wall-clock timeout (seconds, 1..=3600) for this command before it is killed. Use a larger value for long-running work (builds, test suites, solvers). Omitted or out-of-range falls back to the configured tool timeout."
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

    /// Shell runs scripts that legitimately take a long time, so it runs
    /// unbounded unless the caller passes an explicit `timeout_secs`. This
    /// keeps the harness from hard-killing a long command at the global tool
    /// timeout (issue #4023).
    fn timeout_policy(&self, args: &serde_json::Value) -> ToolTimeout {
        match args.get("timeout_secs").and_then(|v| v.as_u64()) {
            // `0` (or absent) means "no deadline".
            None | Some(0) => ToolTimeout::Unbounded,
            Some(secs) => ToolTimeout::Secs(secs),
        }
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

        // Optional per-call wall-clock budget. `None`/`0` ⇒ run unbounded;
        // a positive value is clamped downstream by
        // `tool_timeout::explicit_call_timeout_*`. Shell has no default deadline
        // (issue #4023) — long scripts must run to completion.
        let requested_timeout = args.get("timeout_secs").and_then(|v| v.as_u64());

        let start = Instant::now();
        let (allowed, result) = self.run_with_security(command, requested_timeout).await;
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
    pub(crate) async fn run_with_security(
        &self,
        command: &str,
        requested_timeout: Option<u64>,
    ) -> (bool, ToolResult) {
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

        // When the agent's sandbox mode is `Sandboxed`, route execution
        // through the sandbox backend (Docker or OS-level jail) instead
        // of the normal runtime. Security checks above still apply.
        if matches!(
            crate::openhuman::agent::harness::current_sandbox_mode(),
            Some(crate::openhuman::agent::harness::definition::SandboxMode::Sandboxed)
        ) {
            return self.run_sandboxed(command, requested_timeout).await;
        }

        // Execute with timeout to prevent hanging commands.
        // Clear the environment to prevent leaking API keys and other secrets
        // (CWE-200), then re-add only safe, functional variables.
        let mut cmd = match self
            .runtime
            .build_shell_command(command, &self.effective_action_dir())
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

        // Point the child's temp dir at the agent's granted scratch dir
        // (`/tmp/openhuman`, a ReadWrite trusted root — see SecurityPolicy
        // `from_config`) so `python3 tempfile` / `mktemp` / `$TMPDIR` writes land
        // in a sandboxed, readable location instead of the world-shared /tmp.
        let scratch_dir = crate::openhuman::security::openhuman_scratch_dir();
        if scratch_dir.is_dir() {
            tracing::debug!(
                scratch_dir = %scratch_dir.display(),
                "[shell] overriding TMPDIR/TMP/TEMP to the openhuman scratch dir"
            );
            cmd.env("TMPDIR", scratch_dir.as_os_str());
            cmd.env("TMP", scratch_dir.as_os_str());
            cmd.env("TEMP", scratch_dir.as_os_str());
        } else {
            tracing::debug!(
                scratch_dir = %scratch_dir.display(),
                "[shell] scratch dir missing — leaving TMPDIR/TMP/TEMP as inherited"
            );
        }

        match self.runtime_path_for_command(command).await {
            Ok(Some(path)) => {
                tracing::debug!(path = %path, "[shell] applying managed runtime PATH");
                cmd.env("PATH", path);
            }
            Ok(None) => {}
            Err(error) => {
                return (
                    true,
                    ToolResult::error(format!("Failed to resolve command runtime: {error}")),
                );
            }
        }

        // No default deadline — only a caller-supplied `timeout_secs` bounds the
        // run. `None` ⇒ run to completion (issue #4023).
        let explicit_timeout = self.explicit_timeout(requested_timeout);
        tracing::debug!(
            timeout_secs = ?explicit_timeout.map(|d| d.as_secs()),
            requested_timeout_secs = ?requested_timeout,
            "[shell] starting command ({} timeout)",
            if explicit_timeout.is_some() { "explicit" } else { "no" }
        );
        let result = match explicit_timeout {
            Some(timeout) => tokio::time::timeout(timeout, cmd.output()).await,
            None => Ok(cmd.output().await),
        };

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
                    // Surface the exit code AND both streams so the agent can
                    // diagnose the failure (e.g. 127 missing dependency, 126
                    // sandbox/permission wall) instead of looping on it (#4095).
                    super::command_output::command_failure(output.status.code(), &stdout, &stderr)
                }
            }
            Ok(Err(e)) => ToolResult::error(format!("Failed to execute command: {e}")),
            Err(_) => ToolResult::error(format!(
                "Command timed out after {}s and was killed",
                explicit_timeout.map(|d| d.as_secs()).unwrap_or(0)
            )),
        };
        (true, tool_result)
    }

    /// Execute a command through the sandbox backend. Called when the
    /// agent's `SandboxMode` is `Sandboxed`.
    async fn run_sandboxed(
        &self,
        command: &str,
        requested_timeout: Option<u64>,
    ) -> (bool, ToolResult) {
        use crate::openhuman::sandbox;

        let config = crate::openhuman::config::RuntimeConfig::default();
        let policy = sandbox::resolve_sandbox_policy(
            crate::openhuman::agent::harness::definition::SandboxMode::Sandboxed,
            &self.effective_action_dir(),
            &config,
            false,
        );

        tracing::debug!(
            backend = ?policy.backend,
            command = command,
            "[shell] routing to sandbox backend"
        );

        let mut extra_env = std::collections::HashMap::new();
        match self.runtime_path_for_command(command).await {
            Ok(Some(path)) => {
                extra_env.insert("PATH".to_string(), path);
            }
            Ok(None) => {}
            Err(error) => {
                return (
                    true,
                    ToolResult::error(format!("Failed to resolve command runtime: {error}")),
                );
            }
        }

        // Sandbox backends require a finite deadline. Without an explicit
        // `timeout_secs`, substitute the generous effective-unbounded cap so a
        // long command isn't killed while still bounding a wedged sandbox.
        let explicit_timeout = self.explicit_timeout(requested_timeout);
        let effective = explicit_timeout.unwrap_or_else(|| {
            Duration::from_secs(crate::openhuman::tool_timeout::SANDBOX_UNBOUNDED_CAP_SECS)
        });
        tracing::debug!(
            timeout_secs = effective.as_secs(),
            requested_timeout_secs = ?requested_timeout,
            unbounded = explicit_timeout.is_none(),
            "[shell] starting sandboxed command"
        );

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
                let tool_result = if result.timed_out {
                    ToolResult::error(format!(
                        "Command timed out after {}s and was killed",
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
                    // Same exit-code + both-streams surfacing as the native path
                    // (#4095); the sandbox `-1` sentinel renders as a signal.
                    super::command_output::command_failure(
                        super::command_output::sandbox_exit_code(result.exit_code),
                        &result.stdout,
                        &result.stderr,
                    )
                };
                (true, tool_result)
            }
            Err(e) => (
                true,
                ToolResult::error(format!("Sandbox execution failed: {e}")),
            ),
        }
    }

    async fn runtime_path_for_command(&self, command: &str) -> anyhow::Result<Option<String>> {
        let mut prepend_dirs = Vec::new();

        // Node injection preserves the existing contract: shell only sees the
        // managed Node bin directory after a previous node/npm tool resolved it.
        if let Some(bootstrap) = self.node_bootstrap.as_ref() {
            if let Some(resolved) = bootstrap.try_cached() {
                tracing::debug!(
                    bin_dir = %resolved.bin_dir.display(),
                    version = %resolved.version,
                    "[shell] prepending managed node bin to PATH"
                );
                prepend_dirs.push(resolved.bin_dir);
            }
        }

        if shell_command_needs_python_runtime(command) {
            if let Some(bootstrap) = self.python_bootstrap.as_ref() {
                let resolved = bootstrap.resolve().await?;
                tracing::debug!(
                    bin_dir = %resolved.bin_dir.display(),
                    python_bin = %resolved.python_bin.display(),
                    version = %resolved.version,
                    source = ?resolved.source,
                    "[shell] prepending python runtime bin to PATH"
                );
                prepend_dirs.push(resolved.bin_dir);
            }
        }

        if prepend_dirs.is_empty() {
            Ok(None)
        } else {
            Ok(Some(prepend_path_dirs(
                prepend_dirs.iter().map(|p| p.as_path()),
                &std::env::var("PATH").unwrap_or_default(),
            )))
        }
    }
}

fn prepend_path_dirs<'a>(
    dirs: impl IntoIterator<Item = &'a std::path::Path>,
    host_path: &str,
) -> String {
    let sep = if cfg!(windows) { ";" } else { ":" };
    let mut parts: Vec<String> = dirs
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    if !host_path.is_empty() {
        parts.push(host_path.to_string());
    }
    parts.join(sep)
}

fn shell_command_needs_python_runtime(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    lower
        .split(|ch| matches!(ch, ';' | '&' | '|' | '\n' | '\r'))
        .any(segment_starts_with_python_command)
}

fn segment_starts_with_python_command(segment: &str) -> bool {
    let mut tokens = segment.split_whitespace().peekable();
    while let Some(token) = tokens.next() {
        let token = token.trim_matches(|ch| matches!(ch, '(' | ')' | '<' | '>'));
        if token.is_empty() {
            continue;
        }
        if token.contains('=') && !token.starts_with('-') {
            continue;
        }
        if matches!(token, "sudo" | "command" | "time" | "env") {
            continue;
        }
        return is_python_executable_token(token);
    }
    false
}

fn is_python_executable_token(token: &str) -> bool {
    let executable = token.rsplit('/').next().unwrap_or(token);
    matches!(
        executable,
        "python"
            | "python3"
            | "py"
            | "pip"
            | "pip3"
            | "python.exe"
            | "python3.exe"
            | "pip.exe"
            | "pip3.exe"
    ) || versioned_executable(executable, "python3.")
        || versioned_executable(executable, "pip3.")
}

fn versioned_executable(executable: &str, prefix: &str) -> bool {
    executable
        .strip_prefix(prefix)
        .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
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
            action_dir: std::env::temp_dir(),
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

    /// End-to-end regression guard for #3238.
    ///
    /// PR #3074 split `Config.action_dir` (the agent's read/write root)
    /// from `Config.workspace_dir` (internal product state). `ShellTool`
    /// is contractually obligated to spawn its child process with
    /// `current_dir = security.action_dir` so `pwd` inside the shell
    /// reports the action sandbox path, never `workspace_dir` and never
    /// the cargo-test caller's CWD.
    ///
    /// This test constructs a `SecurityPolicy` whose `action_dir` is a
    /// fresh tempdir (distinct from `workspace_dir` and from `cwd`),
    /// runs `pwd`, and asserts the captured stdout canonicalises to the
    /// same path as `action_dir`. If `ShellTool::run_with_security`
    /// stops passing `&security.action_dir` to `build_shell_command`
    /// (or `build_shell_command` stops calling `current_dir`), this
    /// test fails before the regression ships.
    #[cfg(not(windows))]
    #[tokio::test]
    async fn shell_pwd_returns_action_dir_not_workspace_dir() {
        let action_tmp = tempfile::tempdir().expect("create action tempdir");
        let workspace_tmp = tempfile::tempdir().expect("create workspace tempdir");
        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: workspace_tmp.path().to_path_buf(),
            action_dir: action_tmp.path().to_path_buf(),
            ..SecurityPolicy::default()
        });
        let tool = ShellTool::new(security.clone(), test_runtime(), test_audit());

        let result = tool
            .execute(json!({"command": "pwd"}))
            .await
            .expect("pwd should execute without harness error");
        assert!(
            !result.is_error,
            "pwd unexpectedly errored: {}",
            result.output()
        );

        // Canonicalise both sides — on macOS `/tmp` is a symlink to
        // `/private/tmp`, so the raw strings won't match even when the
        // paths are the same.
        let reported = std::path::PathBuf::from(result.output().trim());
        let actual = reported.canonicalize().unwrap_or_else(|_| reported.clone());
        let expected = security
            .action_dir
            .canonicalize()
            .unwrap_or_else(|_| security.action_dir.clone());
        let workspace_canon = security
            .workspace_dir
            .canonicalize()
            .unwrap_or_else(|_| security.workspace_dir.clone());

        assert_eq!(
            actual,
            expected,
            "pwd must report `action_dir`. got `{}`, expected `{}`. \
             If this fails, `ShellTool::run_with_security` likely stopped \
             passing `&security.action_dir` to `runtime.build_shell_command`, \
             or `build_shell_command` stopped calling `current_dir(...)`. See #3238.",
            actual.display(),
            expected.display(),
        );
        assert_ne!(
            actual, workspace_canon,
            "pwd reported `workspace_dir` instead of `action_dir` — the \
             action/internal split is broken. See #3074, #3238."
        );
    }

    /// Source-level regression guard for #3238.
    ///
    /// Locks in the contract that the three shell-family acting tools
    /// (`shell`, `node_exec`, `npm_exec`) resolve their CWD against
    /// `security.action_dir`, never `security.workspace_dir`. The
    /// behavioural assertion above covers `shell`; this guard catches
    /// regressions in `node_exec` / `npm_exec` without requiring a real
    /// Node.js install in CI (their `execute()` path runs
    /// `NodeBootstrap::resolve()` first, which is brittle to mock).
    ///
    /// If a future refactor accidentally switches any of these tools
    /// back to `workspace_dir`, this assertion fires at compile-time
    /// string-match level.
    #[test]
    fn shell_family_tools_route_cwd_through_action_dir() {
        const SHELL_SRC: &str = include_str!("shell.rs");
        const NODE_EXEC_SRC: &str = include_str!("node_exec.rs");
        const NPM_EXEC_SRC: &str = include_str!("npm_exec.rs");

        // Compose forbidden patterns at runtime so this test's own source
        // doesn't trigger the contains() check on itself.
        let bad_field = format!("self.security.{}_dir", "workspace");
        let bad_call_1 = format!("build_shell_command(&command, &{bad_field})");
        let bad_call_2 = format!("build_shell_command(command, &{bad_field})");

        for (name, src) in [
            ("shell.rs", SHELL_SRC),
            ("node_exec.rs", NODE_EXEC_SRC),
            ("npm_exec.rs", NPM_EXEC_SRC),
        ] {
            assert!(
                src.contains("self.security.action_dir"),
                "{name} must reference `self.security.action_dir` for tool CWD \
                 (see #3074, #3238)"
            );
            assert!(
                !src.contains(&bad_call_1) && !src.contains(&bad_call_2),
                "{name} must not pass `workspace_dir` to `build_shell_command` — \
                 acting tools spawn into `action_dir`. See #3074, #3238."
            );
        }
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

    /// Regression for the code_executor no-progress loop (#4095): a FAILED
    /// command must surface its exit code AND both streams — never drop stdout
    /// when stderr is present — so the agent can read *why* it failed instead of
    /// re-running it blindly.
    #[cfg(not(windows))]
    #[tokio::test]
    async fn shell_failure_surfaces_exit_code_and_both_streams() {
        let tool = ShellTool::new(
            test_security(AutonomyLevel::Full),
            test_runtime(),
            test_audit(),
        );
        let result = tool
            .execute(json!({
                "command": "echo stdout-marker; echo stderr-marker 1>&2; exit 7"
            }))
            .await
            .unwrap();
        assert!(result.is_error, "non-zero exit must be an error result");
        let out = result.output();
        assert!(out.contains("exit code 7"), "exit code not surfaced: {out}");
        assert!(
            out.contains("stdout-marker"),
            "stdout dropped on failure: {out}"
        );
        assert!(
            out.contains("stderr-marker"),
            "stderr dropped on failure: {out}"
        );
    }

    /// A missing executable exits 127; the surfaced result must carry the code
    /// and the actionable "command not found / missing dependency" hint so the
    /// agent recognises the dependency wall and adapts instead of looping.
    #[cfg(not(windows))]
    #[tokio::test]
    async fn shell_missing_command_surfaces_127_with_dependency_hint() {
        let tool = ShellTool::new(
            test_security(AutonomyLevel::Full),
            test_runtime(),
            test_audit(),
        );
        let result = tool
            .execute(json!({"command": "this_binary_does_not_exist_xyz --version"}))
            .await
            .unwrap();
        assert!(result.is_error);
        let out = result.output().to_lowercase();
        assert!(out.contains("127"), "exit code 127 not surfaced: {out}");
        assert!(
            out.contains("command not found"),
            "missing-dependency hint absent: {out}"
        );
    }

    fn test_security_with_env_cmd() -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: std::env::temp_dir(),
            action_dir: std::env::temp_dir(),
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
    fn shell_is_unbounded_by_default() {
        let tool = ShellTool::new(
            test_security(AutonomyLevel::Supervised),
            test_runtime(),
            test_audit(),
        );

        // No `timeout_secs` ⇒ no deadline. A long script must not be hard-killed.
        assert_eq!(
            tool.timeout_policy(&json!({"command": "make"})),
            ToolTimeout::Unbounded
        );
        assert_eq!(tool.explicit_timeout(None), None);
        // An explicit 0 disables the timeout too.
        assert_eq!(
            tool.timeout_policy(&json!({"command": "make", "timeout_secs": 0})),
            ToolTimeout::Unbounded
        );
        assert_eq!(tool.explicit_timeout(Some(0)), None);
    }

    #[test]
    fn shell_timeout_honors_explicit_per_call_value() {
        let tool = ShellTool::new(
            test_security(AutonomyLevel::Supervised),
            test_runtime(),
            test_audit(),
        );

        // An explicit in-range request is enforced verbatim.
        assert_eq!(
            tool.timeout_policy(&json!({"command": "make", "timeout_secs": 1800})),
            ToolTimeout::Secs(1800)
        );
        assert_eq!(
            tool.explicit_timeout(Some(1800)),
            Some(Duration::from_secs(1800))
        );

        // Above the cap clamps down to MAX_TIMEOUT_SECS (3600).
        assert_eq!(
            tool.explicit_timeout(Some(9_999)),
            Some(Duration::from_secs(
                crate::openhuman::tool_timeout::MAX_TIMEOUT_SECS
            ))
        );
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn shell_per_call_timeout_kills_slow_command() {
        let tool = ShellTool::new(
            test_security(AutonomyLevel::Supervised),
            test_runtime(),
            test_audit(),
        );

        // `sleep 30` would survive any sane default, but a per-call 1s budget
        // must kill it and report the per-call value in the error message.
        let result = tool
            .execute(json!({"command": "sleep 30", "timeout_secs": 1}))
            .await
            .unwrap();

        assert!(result.is_error, "slow command should time out");
        let text = result.text();
        assert!(
            text.contains("timed out after 1s"),
            "timeout message should reflect the per-call budget, got: {text}"
        );
    }

    #[test]
    fn shell_schema_advertises_timeout_secs() {
        let tool = ShellTool::new(
            test_security(AutonomyLevel::Supervised),
            test_runtime(),
            test_audit(),
        );
        let schema = tool.parameters_schema();
        let timeout = &schema["properties"]["timeout_secs"];
        assert_eq!(timeout["type"], "integer");
        assert_eq!(timeout["minimum"], 1);
        assert_eq!(timeout["maximum"], 3600);
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

    #[test]
    fn shell_detects_python_runtime_commands() {
        for command in [
            "python3 -m pyfiglet hello",
            "python -m pip install pyfiglet",
            "pip install pyfiglet",
            "pip3.13 show pyfiglet",
            "/opt/openhuman/python/bin/python3 script.py",
            "echo hi && python3 -V",
        ] {
            assert!(
                shell_command_needs_python_runtime(command),
                "expected python runtime detection for {command}"
            );
        }

        for command in [
            "echo python3",
            "ls",
            "cat ./pipelines.txt",
            "node script.js",
        ] {
            assert!(
                !shell_command_needs_python_runtime(command),
                "did not expect python runtime detection for {command}"
            );
        }
    }

    #[test]
    fn shell_runtime_path_prepends_managed_dirs_before_host_path() {
        let python = std::path::Path::new("/opt/openhuman/python/bin");
        let node = std::path::Path::new("/opt/openhuman/node/bin");
        let joined = prepend_path_dirs([python, node], "/usr/local/bin:/usr/bin");
        let sep = if cfg!(windows) { ";" } else { ":" };
        assert_eq!(
            joined,
            format!(
                "{}{}{}{}{}",
                python.display(),
                sep,
                node.display(),
                sep,
                "/usr/local/bin:/usr/bin"
            )
        );
    }

    /// Empirical answer to "does `shell` resolve/install managed Node on its
    /// own?" — NO. The shell path consults the managed Node bootstrap only via
    /// `try_cached()`, which never calls `resolve()` and therefore never
    /// downloads/installs anything. So without a prior `node_exec` / `npm_exec`
    /// (the tools that DO call `resolve()` and share this bootstrap instance),
    /// `runtime_path_for_command` injects nothing for a node command. On a host
    /// with no Node in the login PATH, the command then fails — the managed
    /// runtime is never reached on the shell path. (Python, by contrast, IS
    /// self-resolved in `runtime_path_for_command` — see the python branch.)
    #[tokio::test]
    async fn shell_does_not_resolve_or_install_node_on_its_own() {
        let node = Arc::new(NodeBootstrap::new(
            crate::openhuman::config::schema::NodeConfig {
                enabled: true,
                version: "v22.11.0".to_string(),
                cache_dir: String::new(),
                prefer_system: true,
            },
            std::env::temp_dir(),
            reqwest::Client::new(),
        ));
        let tool = ShellTool::with_language_bootstraps(
            test_security(AutonomyLevel::Full),
            test_runtime(),
            test_audit(),
            Some(node),
            None,
        );

        // Unprimed (no prior node_exec/npm_exec resolve): shell injects NO
        // managed node bin onto PATH — it does not auto-resolve or install.
        let injected = tool
            .runtime_path_for_command("node --version")
            .await
            .expect("runtime path resolves");
        assert!(
            injected.is_none(),
            "shell injected a managed node bin without any prior node_exec/npm_exec \
             resolve — it must not auto-resolve/install on the shell path: {injected:?}"
        );
    }

    #[tokio::test]
    async fn shell_blocks_rate_limited() {
        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            max_actions_per_hour: 0,
            workspace_dir: std::env::temp_dir(),
            action_dir: std::env::temp_dir(),
            ..SecurityPolicy::default()
        });
        let tool = ShellTool::new(security, test_runtime(), test_audit());
        let result = tool.execute(json!({"command": "echo test"})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("Rate limit"));
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn shell_sandboxed_mode_routes_through_sandbox_backend() {
        use crate::openhuman::agent::harness::definition::SandboxMode;
        use crate::openhuman::agent::harness::with_current_sandbox_mode;

        let tool = ShellTool::new(
            test_security(AutonomyLevel::Supervised),
            test_runtime(),
            test_audit(),
        );
        let result = with_current_sandbox_mode(SandboxMode::Sandboxed, async {
            tool.execute(json!({"command": "echo sandboxed-output"}))
                .await
                .unwrap()
        })
        .await;
        assert!(
            !result.is_error,
            "sandboxed echo should succeed: {}",
            result.output()
        );
        assert!(
            result.output().contains("sandboxed-output"),
            "expected 'sandboxed-output' in result, got: {:?}",
            result.output()
        );
    }

    /// Regression guard for #3235 (cwd_jail wiring for shell-family tools).
    ///
    /// PR #3261 wired `ShellTool` to route through `sandbox::execute_in_sandbox`
    /// (which uses `cwd_jail` for the local-OS-jail backend) when the
    /// active agent's `SandboxMode::Sandboxed` is set. This PR extends the
    /// same wiring to `NodeExecTool` and `NpmExecTool`. The behavioural
    /// `shell_sandboxed_mode_routes_through_sandbox_backend` test above
    /// proves the contract end-to-end for `shell` (no managed-Node
    /// dependency); `node_exec` and `npm_exec` cannot run end-to-end in
    /// unit tests without a resolved `NodeBootstrap`, so this source-grep
    /// guard catches refactors that drop the sandbox check from either
    /// tool's `execute()` body.
    #[test]
    fn shell_family_tools_route_to_sandbox_when_sandboxed_mode_active() {
        const SHELL_SRC: &str = include_str!("shell.rs");
        const NODE_EXEC_SRC: &str = include_str!("node_exec.rs");
        const NPM_EXEC_SRC: &str = include_str!("npm_exec.rs");

        for (name, src) in [
            ("shell.rs", SHELL_SRC),
            ("node_exec.rs", NODE_EXEC_SRC),
            ("npm_exec.rs", NPM_EXEC_SRC),
        ] {
            assert!(
                src.contains("current_sandbox_mode()"),
                "{name} must check `current_sandbox_mode()` to detect SandboxMode::Sandboxed \
                 sessions and route through the sandbox backend (see #3235)"
            );
            assert!(
                src.contains("SandboxMode::Sandboxed"),
                "{name} must compare against `SandboxMode::Sandboxed` to opt in to the \
                 sandbox routing path (see #3235)"
            );
            // Use the call-site pattern `.run_sandboxed(` so the assertion
            // doesn't trivially pass on the helper definition itself
            // (`fn run_sandboxed(...)`). If `execute()` / `run_with_security()`
            // stop delegating, this fires even though the helper still exists.
            assert!(
                src.contains(".run_sandboxed("),
                "{name} must delegate to a `run_sandboxed` helper when the sandbox mode is \
                 active (see #3235). Whitespace before `.run_sandboxed` is tolerated; the \
                 helper call must appear in the source — *not* just the helper definition."
            );
        }
    }
}
