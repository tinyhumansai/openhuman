use super::shell_description::{command_param_description, shell_description};
use super::shell_run::{audit_once, run_in_policy, run_native, AuditOnce, YieldRequest};
#[cfg(feature = "modules")]
use super::shell_support::container_environment;
use super::shell_support::{command_failure, tool_timeout_message};
use crate::agent::tinyagents::host::run_context::{
    command_environment_for_tool, dispatch_ctx_for_tool,
};
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::modules::tinybox::host_runtime::RuntimeAdapter;
use crate::security::audit_record::HostAuditEvent;
use crate::security::{AuditSink, SecurityPolicy};
use async_trait::async_trait;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};
use tinytools::ToolRunContext;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult, ToolTimeout};

/// Maximum output size in bytes (1MB).
const MAX_OUTPUT_BYTES: usize = 1_048_576;
#[cfg(feature = "modules")]
fn docker_module_result(
    output: crate::modules::tinybox::contract::DockerRunOutput,
    explicit_timeout: Option<Duration>,
) -> ToolResult {
    let mut stdout = output.stdout;
    let mut stderr = output.stderr;
    if stdout.len() > MAX_OUTPUT_BYTES {
        stdout.truncate(crate::util::floor_char_boundary(&stdout, MAX_OUTPUT_BYTES));
        stdout.push_str("\n... [output truncated at 1MB]");
    }
    if stderr.len() > MAX_OUTPUT_BYTES {
        stderr.truncate(crate::util::floor_char_boundary(&stderr, MAX_OUTPUT_BYTES));
        stderr.push_str("\n... [stderr truncated at 1MB]");
    }

    if output.timed_out {
        let secs = explicit_timeout
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let mut message = tool_timeout_message(secs);
        if !stdout.is_empty() {
            message.push_str("\n[stdout]\n");
            message.push_str(&stdout);
        }
        if !stderr.is_empty() {
            message.push_str("\n[stderr]\n");
            message.push_str(&stderr);
        }
        ToolResult::error(message)
    } else if output.exit_code == 0 {
        if stderr.is_empty() {
            ToolResult::success(stdout)
        } else {
            ToolResult::success(format!("{stdout}\n[stderr]\n{stderr}"))
        }
    } else {
        command_failure(Some(output.exit_code), &stdout, &stderr)
    }
}

/// Shell command execution tool with sandboxing
pub struct ShellTool {
    security: Arc<SecurityPolicy>,
    runtime: Arc<dyn RuntimeAdapter>,
    audit: Arc<AuditSink>,
    analysis_config: Option<Arc<Config>>,
}

impl ShellTool {
    pub fn new(
        security: Arc<SecurityPolicy>,
        runtime: Arc<dyn RuntimeAdapter>,
        audit: Arc<AuditSink>,
    ) -> Self {
        Self {
            security,
            runtime,
            audit,
            analysis_config: None,
        }
    }

    /// Builds a shell tool whose command facts come from the configured
    /// TinyBox module before host authorization and execution.
    pub fn with_config(
        security: Arc<SecurityPolicy>,
        runtime: Arc<dyn RuntimeAdapter>,
        audit: Arc<AuditSink>,
        config: Arc<Config>,
    ) -> Self {
        Self {
            security,
            runtime,
            audit,
            analysis_config: Some(config),
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
    /// `tinyagents_harness_tools::exec::timeout::explicit_call_timeout_duration`.
    fn explicit_timeout(&self, requested: Option<u64>) -> Option<Duration> {
        tinyagents_harness_tools::exec::timeout::explicit_call_timeout_duration(
            requested,
            tinyagents_harness_tools::exec::timeout::MAX_TIMEOUT_SECS,
        )
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        shell_description(self.runtime.shell_flavor())
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": command_param_description(self.runtime.shell_flavor())
                },
                "category": {
                    "type": "string",
                    "enum": ["read", "write", "network", "install", "destructive"],
                    "description": "Optional self-declared risk; can only raise the approval requirement."
                },
                "timeout_secs": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 3600,
                    "description": "Optional: how long (seconds) to wait before coming back. A command still running then returns its output so far and keeps running as a process you follow with `shell_process`. Default 120, at most 600; 0 waits for the command to end. Only where a command cannot be left running (four already are, or a Docker sandbox) is it stopped at this time instead. To stop a command after a time, put `timeout N` in the command."
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
            Some(secs) => ToolTimeout::Millis(secs.saturating_mul(1000)),
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
        // This synchronous declaration cannot consult TinyBox. Be
        // conservative here; the host authorization gate uses the exact
        // per-call TinyBox analysis and applies the configured tier.
        !command.is_empty()
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_in_context(&dispatch_ctx_for_tool(None), args, None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.execute_in_context(&dispatch_ctx_for_tool(context), args, context)
            .await
    }
}

impl ShellTool {
    async fn execute_in_context(
        &self,
        ctx: &DispatchCtx,
        args: serde_json::Value,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'command' parameter"))?;

        // The command's deadline: in a turn, what TinyAgents' turn-budget clamp
        // wrote (the model's own `timeout_secs` is its yield point, moved to
        // `YIELD_AFTER_ARG` by `middleware::shell_yield`); outside a turn, the
        // caller's. `None`/`0` ⇒ unbounded (issue #4023).
        let requested_timeout = args.get("timeout_secs").and_then(|v| v.as_u64());
        let come_back = YieldRequest::from_args(&args);
        // Whether a human approved the call is the harness ApprovalGate's
        // decision, which the gate records separately; that yes/no is not
        // threaded into tools, so this row never claims an approval
        // (`approved=false`). A command left running is audited when it ends,
        // with its real outcome.
        let approved = false;
        let audit = audit_once(
            self.audit.clone(),
            ctx.clone(),
            self.analysis_config.clone(),
            command.to_owned(),
            approved,
        );
        let start = Instant::now();
        let (allowed, result) = crate::sandbox::shell_progress::with_shell_progress(
            context,
            crate::sandbox::shell_progress::HEARTBEAT_INTERVAL,
            |meter| {
                self.run_with_security_in_context(
                    ctx,
                    command,
                    requested_timeout,
                    come_back,
                    context,
                    meter,
                    &audit,
                )
            },
        )
        .await;
        let duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        // `allowed` = passed the in-tool security checks.
        let hook = audit.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(hook) = hook {
            hook(allowed, !result.is_error, duration_ms);
        }
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
        ctx: &DispatchCtx,
        command: &str,
        requested_timeout: Option<u64>,
    ) -> (bool, ToolResult) {
        let come_back = YieldRequest::default();
        let audit = std::sync::Mutex::new(None);
        self.run_with_security_in_context(
            ctx,
            command,
            requested_timeout,
            come_back,
            None,
            None,
            &audit,
        )
        .await
    }

    async fn run_with_security_in_context(
        &self,
        ctx: &DispatchCtx,
        command: &str,
        requested_timeout: Option<u64>,
        come_back: YieldRequest,
        context: Option<&dyn ToolRunContext>,
        meter: Option<crate::sandbox::shell_progress::ShellOutputMeter>,
        audit: &AuditOnce,
    ) -> (bool, ToolResult) {
        let command_environment = command_environment_for_tool(context);
        // TinySecurity owns command classification and admission. Re-check at
        // execution time so a command approved earlier cannot bypass a policy
        // reload or a newly exhausted budget.
        let verdict = match self
            .security
            .check_command(crate::security::policy::CommandQuery {
                command,
                declared: None,
                flow: crate::security::policy::CommandFlow::Harness,
                stage: crate::security::policy::CommandStage::Execute,
            })
            .await
        {
            Ok(verdict) if verdict.gate != crate::security::policy::GateDecision::Block => verdict,
            Ok(_) => {
                return (
                    false,
                    ToolResult::error("[policy-blocked] Security policy blocked this command."),
                );
            }
            Err(reason) => return (false, ToolResult::error(reason.to_string())),
        };
        let _ = verdict;

        let _cwd = self.effective_action_dir_for_context(context);

        // SaaS: every command runs in the user's container, whatever the
        // agent's sandbox mode says, and never on the host.
        let saas_action_dir = self.effective_action_dir_for_context(context);
        if let Some(resolved) =
            super::shell_saas::saas_sandbox_with(crate::core::runtime::is_saas(), || {
                crate::profiles::tools::sandbox_policy(
                    &saas_action_dir,
                    &self.security.workspace_dir,
                )
            })
        {
            let action_dir = saas_action_dir;
            return match resolved {
                Ok(policy) => {
                    let (timeout, dir) = (requested_timeout, &action_dir);
                    run_in_policy(
                        ctx, policy, command, timeout, come_back, dir, context, meter, audit,
                    )
                    .await
                }
                Err(why) => {
                    tracing::warn!(reason = %why, "[shell] SaaS sandbox refused the command");
                    // Nothing ran: report it as not allowed.
                    super::shell_saas::saas_sandbox_refusal(&why)
                }
            };
        }

        // A sandboxed agent or an explicit operator backend uses the sandbox
        // execution path. Security checks above still apply.
        let sandbox_required = match crate::sandbox::policy::should_plan_command(
            ctx,
            crate::agent::tinyagents::host::run_context::sandbox_mode_for_tool(context),
        )
        .await
        {
            Ok(required) => required,
            Err(err) => {
                return (
                    false,
                    ToolResult::error(format!("Cannot read sandbox configuration: {err}")),
                );
            }
        };
        if sandbox_required {
            let action_dir = self.effective_action_dir_for_context(context);
            let timeout = requested_timeout;
            return self
                .run_sandboxed(
                    ctx,
                    command,
                    timeout,
                    come_back,
                    &action_dir,
                    context,
                    meter,
                    audit,
                )
                .await;
        }

        // Clear the environment to prevent leaking API keys and other secrets
        // (CWE-200), then re-add only safe, functional variables.
        let action_dir = self.effective_action_dir_for_context(context);
        #[cfg(feature = "modules")]
        if let Some((mut request, mount_workspace)) =
            self.runtime.build_docker_request(command, &action_dir)
        {
            let Some(config) = self.analysis_config.as_deref() else {
                return (
                    false,
                    ToolResult::error("Docker module execution is unavailable."),
                );
            };

            // Docker cannot leave a command running: the model's come-back
            // time stays its deadline here.
            let explicit_timeout =
                self.explicit_timeout(come_back.deadline_instead(requested_timeout));
            if let Some(timeout) = explicit_timeout {
                let timeout_ms = timeout
                    .as_nanos()
                    .saturating_add(999_999)
                    .checked_div(1_000_000)
                    .unwrap_or(0)
                    .max(1);
                let Ok(timeout_ms) = u64::try_from(timeout_ms) else {
                    return (
                        false,
                        ToolResult::error("Docker timeout exceeds the supported limit."),
                    );
                };
                if timeout_ms > crate::modules::tinybox::contract::MAX_DOCKER_RUN_TIMEOUT_MS {
                    return (
                        false,
                        ToolResult::error("Docker timeout exceeds the supported limit."),
                    );
                }
                request.timeout_ms = timeout_ms;
            }

            let Some(mut env) = container_environment(command_environment.as_deref()) else {
                return (
                    false,
                    ToolResult::error("Docker environment contains a non-UTF-8 value."),
                );
            };
            for (name, value) in tinyagents_harness_tools::fs::filesystem::shell_git_env() {
                let (Ok(name), Ok(value)) = (name.into_string(), value.into_string()) else {
                    return (
                        false,
                        ToolResult::error("Docker environment contains a non-UTF-8 value."),
                    );
                };
                env.insert(name, value);
            }

            let scratch_dir = crate::security::openhuman_scratch_dir();
            if scratch_dir.is_dir() {
                let Some(scratch_dir) = scratch_dir.to_str() else {
                    return (
                        false,
                        ToolResult::error("Docker scratch directory is not valid UTF-8."),
                    );
                };
                for name in ["TMPDIR", "TMP", "TEMP"] {
                    env.insert(name.to_owned(), scratch_dir.to_owned());
                }
            }
            request.env = env;

            let result = if mount_workspace {
                crate::modules::tinybox::run_docker(config, request).await
            } else {
                crate::modules::tinybox::run_docker_without_workspace(config, request).await
            };
            let result = match result {
                Ok(output) => docker_module_result(output, explicit_timeout),
                Err(_) => ToolResult::error("Docker module execution failed."),
            };
            return (true, result);
        }

        // The module spawns the shell: it selects cmd or bash, starts from its
        // own safe environment (no API keys or other secrets, CWE-200) and owns
        // the per-call scratch directory. The host adds only what it approves.
        if let Err(e) = crate::config::ensure_usable_cwd(&action_dir) {
            return (
                true,
                ToolResult::error(format!("Failed to build shell command: {e}")),
            );
        }
        let mut env = std::collections::BTreeMap::new();

        if let Some(overrides) = command_environment.as_deref() {
            env.extend(
                overrides
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
        }

        // A turn that owns its child environment (library agents) keeps it: the
        // module starts from the process snapshot, so the turn's own values for
        // the functional variables are passed as approved overrides.
        if tinyagents_harness_tools::exec::timeout::CommandEnvironment::is_active() {
            for name in crate::sandbox::ops::SANDBOX_ENV_PASSTHROUGH {
                if let Some(value) =
                    tinyagents_harness_tools::exec::timeout::CommandEnvironment::var_os(name)
                {
                    let Ok(value) = value.into_string() else {
                        return (
                            false,
                            ToolResult::error("Shell environment contains a non-UTF-8 value."),
                        );
                    };
                    env.insert((*name).to_owned(), value);
                }
            }
        }

        // Keep command-valued repository Git settings from executing host
        // programs when a shell command happens to invoke git.
        for (key, value) in tinyagents_harness_tools::fs::filesystem::shell_git_env() {
            let (Ok(key), Ok(value)) = (key.into_string(), value.into_string()) else {
                return (
                    false,
                    ToolResult::error("Shell environment contains a non-UTF-8 value."),
                );
            };
            env.insert(key, value);
        }

        super::shell_support::use_scratch_tmp(&mut env);

        // No default deadline — only a caller-supplied `timeout_secs` bounds the
        // run (issue #4023). TinyBox needs a finite budget, so an unbounded call
        // gets the module's ceiling.
        let explicit_timeout = self.explicit_timeout(requested_timeout);
        let effective = explicit_timeout.unwrap_or_else(|| {
            Duration::from_secs(tinyagents_harness_tools::exec::timeout::SANDBOX_UNBOUNDED_CAP_SECS)
        });
        tracing::debug!(
            timeout_secs = effective.as_secs(),
            requested_timeout_secs = ?requested_timeout,
            unbounded = explicit_timeout.is_none(),
            "[shell] starting command through TinyBox"
        );
        let config = match self.analysis_config.as_deref() {
            Some(config) => config.clone(),
            None => match ctx.load_config().await {
                Ok(config) => config,
                Err(e) => {
                    return (
                        true,
                        ToolResult::error(format!("Cannot read module configuration: {e}")),
                    );
                }
            },
        };
        let mut run = crate::sandbox::shell_exec::NativeShellRun::new(
            command,
            &action_dir,
            &self.security.workspace_dir,
            effective,
        );
        run.env = super::shell_support::with_unbuffered_python(env, command_environment.is_some());
        run.python_utf8 = true;
        run.hide_window = self.runtime.hide_window();
        run.meter = meter;
        (
            true,
            run_native(ctx, &config, run, come_back, context, audit).await,
        )
    }

    /// Execute a command through the sandbox backend. Called when the
    /// agent's `SandboxMode` is `Sandboxed`.
    async fn run_sandboxed(
        &self,
        ctx: &DispatchCtx,
        command: &str,
        requested_timeout: Option<u64>,
        come_back: YieldRequest,
        action_dir: &Path,
        context: Option<&dyn ToolRunContext>,
        meter: Option<crate::sandbox::shell_progress::ShellOutputMeter>,
        audit: &AuditOnce,
    ) -> (bool, ToolResult) {
        let config = match ctx.load_config().await {
            Ok(config) => config,
            Err(error) => {
                return (
                    false,
                    ToolResult::error(format!("Cannot read sandbox configuration: {error}")),
                )
            }
        };
        let policy = crate::sandbox::policy::route_for_mode(
            tinyagents_definition::SandboxMode::Sandboxed,
            action_dir,
            &self.security.workspace_dir,
            &config.sandbox,
            &config.runtime,
            false,
        );
        let timeout = requested_timeout;
        run_in_policy(
            ctx, policy, command, timeout, come_back, action_dir, context, meter, audit,
        )
        .await
    }
}

/// Emit a single `CommandExecution` audit event. A write failure is logged as
/// a structured warning but not propagated — audit must never block or fail a
/// tool call, yet a silently broken audit trail must not go unnoticed.
pub(super) fn record_audit(
    audit: &AuditSink,
    ctx: DispatchCtx,
    config: Option<Arc<Config>>,
    command: &str,
    approved: bool,
    allowed: bool,
    success: bool,
    duration_ms: u64,
) {
    if !audit.is_enabled() {
        return;
    }
    let event = HostAuditEvent::command_execution(
        "tool:shell",
        command,
        approved,
        allowed,
        success,
        duration_ms,
    );
    tokio::spawn(async move {
        if let Err(error) =
            crate::security::audit_record::record(&ctx, config.as_deref(), event).await
        {
            tracing::warn!(error = %error, channel = "tool:shell", "[shell] the command execution audit row was not recorded");
        }
    });
}

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;
