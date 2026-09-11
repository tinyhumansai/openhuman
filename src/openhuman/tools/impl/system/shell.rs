use crate::openhuman::agent::host_runtime::RuntimeAdapter;
use crate::openhuman::runtime::javascript::NodeBootstrap;
use crate::openhuman::runtime::python::PythonBootstrap;
use crate::openhuman::security::{AuditLogger, CommandExecutionLog, GateDecision, SecurityPolicy};
use crate::openhuman::tools::traits::{
    PermissionLevel, Tool, ToolCallOptions, ToolResult, ToolTimeout,
};
use async_trait::async_trait;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tinytools::ToolRunContext;

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
    /// Returns the per-worker git-worktree checkout when the tinyagents harness
    /// threaded a [`WorkspaceDescriptor`] into this call's
    /// [`ToolExecutionContext`](tinyagents_harness::tool::ToolExecutionContext) — an edit-capable worker running with
    /// `isolation = "worktree"`, whose isolated worktree root is carried on the
    /// run context (`RunContext::with_workspace`) and surfaced per tool call via
    /// `ToolExecutionContext::from_run_context`. Otherwise falls back to the
    /// shared `self.security.action_dir`, which preserves the non-isolated
    /// behaviour exactly. See #3376, #4249 (08.5).
    fn effective_action_dir_for_context(&self, context: Option<&dyn ToolRunContext>) -> PathBuf {
        if let Some(workspace) = context.and_then(|ctx| ctx.workspace()) {
            tracing::debug!(
                workspace_root = %workspace.root.display(),
                policy_id = %workspace.policy_id,
                "[shell] using TinyAgents workspace descriptor as action dir"
            );
            return workspace.root.clone();
        }
        self.security.action_dir.clone()
    }

    /// The explicit wall-clock budget for this invocation, or `None` to run
    /// unbounded.
    ///
    /// Shell commands run scripts — builds, test suites, solvers — that
    /// legitimately take minutes, so there is **no** default timeout: a
    /// deadline applies only when the caller passes `timeout_secs` (issue
    /// #4023). A `0` explicitly disables it. Any positive value is clamped to
    /// `1..=3600`. See
    /// [`crate::openhuman::tools::timeout::explicit_call_timeout_duration`].
    fn explicit_timeout(&self, requested: Option<u64>) -> Option<Duration> {
        crate::openhuman::tools::timeout::explicit_call_timeout_duration(
            requested,
            crate::openhuman::tools::timeout::MAX_TIMEOUT_SECS,
        )
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "Execute a shell command: run code, manipulate workspace files, or launch applications (`open -a Music`, `xdg-open music://`). Only stdout/stderr comes back, so a script that computes silently or only writes a file returns nothing — print what you need, or read the file afterwards."
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

impl ShellTool {
    async fn execute_in_context(
        &self,
        args: serde_json::Value,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
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
        let (allowed, result) = self
            .run_with_security_in_context(command, requested_timeout, context)
            .await;
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
        self.run_with_security_in_context(command, requested_timeout, None)
            .await
    }

    async fn run_with_security_in_context(
        &self,
        command: &str,
        requested_timeout: Option<u64>,
        context: Option<&dyn ToolRunContext>,
    ) -> (bool, ToolResult) {
        // Read-only `Block` + the Option-2 structural guard. Approval for
        // Write / Network / Destructive already happened at the harness
        // `ApprovalGate` (see `external_effect_with_args`) before `execute()`
        // ran; this enforces what must still hold afterwards.
        if let Err(reason) = self.security.check_gated_command(command) {
            return (false, ToolResult::error(reason));
        }

        // Cross-profile write guard (1b), shell call site. File tools enforce
        // the same boundary per-path in `SecurityPolicy::validate_path`; shell
        // commands never funnel through that, so scan the command's path-shaped
        // tokens against the profile's own workspace (its cwd). No-op unless the
        // session runs under a dedicated-workspace profile. See
        // `profiles::guard::scan_command_for_cross_profile` for the containment
        // rationale (the cwd is already rooted at the profile's own dir).
        let cwd = self.effective_action_dir_for_context(context);
        if let Err(reason) =
            super::check_cross_profile_command(self.security.as_ref(), command, &cwd, "shell")
        {
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
            let action_dir = self.effective_action_dir_for_context(context);
            return self
                .run_sandboxed(command, requested_timeout, &action_dir)
                .await;
        }

        // Execute with timeout to prevent hanging commands.
        // Clear the environment to prevent leaking API keys and other secrets
        // (CWE-200), then re-add only safe, functional variables.
        let action_dir = self.effective_action_dir_for_context(context);
        let mut cmd = match self.runtime.build_shell_command(command, &action_dir) {
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

        // Attribute commits made by this agent-owned shell, without persisting
        // anything in the user's repository or global Git configuration.
        for (key, value) in crate::openhuman::agent::git_attribution::hook_env() {
            cmd.env(key, value);
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
        action_dir: &Path,
    ) -> (bool, ToolResult) {
        use crate::openhuman::sandbox;

        let config = crate::openhuman::config::RuntimeConfig::default();
        let policy = sandbox::resolve_sandbox_policy(
            crate::openhuman::agent::harness::definition::SandboxMode::Sandboxed,
            action_dir,
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
                extra_env.insert("PATH".into(), path.into());
            }
            Ok(None) => {}
            Err(error) => {
                return (
                    true,
                    ToolResult::error(format!("Failed to resolve command runtime: {error}")),
                );
            }
        }

        // The local/no-op sandbox inherits this process's temporary hook
        // directory, so commits made through a sandboxed shell are attributed
        // just like native-shell commits. Docker backends safely ignore an
        // unavailable host path rather than changing repository configuration.
        extra_env.extend(crate::openhuman::agent::git_attribution::hook_env());

        // Sandbox backends require a finite deadline. Without an explicit
        // `timeout_secs`, substitute the generous effective-unbounded cap so a
        // long command isn't killed while still bounding a wedged sandbox.
        let explicit_timeout = self.explicit_timeout(requested_timeout);
        let effective = explicit_timeout.unwrap_or_else(|| {
            Duration::from_secs(crate::openhuman::tools::timeout::SANDBOX_UNBOUNDED_CAP_SECS)
        });
        tracing::debug!(
            timeout_secs = effective.as_secs(),
            requested_timeout_secs = ?requested_timeout,
            unbounded = explicit_timeout.is_none(),
            "[shell] starting sandboxed command"
        );

        match sandbox::execute_in_sandbox(&policy, command, action_dir, extra_env, effective).await
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
        .split([';', '&', '|', '\n', '\r'])
        .any(segment_starts_with_python_command)
}

fn segment_starts_with_python_command(segment: &str) -> bool {
    let tokens = segment.split_whitespace().peekable();
    for token in tokens {
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
#[path = "shell_tests.rs"]
mod tests;
