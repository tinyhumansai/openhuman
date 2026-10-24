//! Running a `shell` call's command, and handing it to the turn when it is
//! still running at its yield point.
//!
//! Both the unsandboxed native shell and the sandbox backends come through
//! here. The native ones (no sandbox, TinyBox's native jail) can leave a
//! command running; Docker cannot, so there the model's own come-back time is
//! a deadline instead, as `timeout_secs` was before it yielded. A call yields
//! only when the turn moved the model's `timeout_secs` to [`YIELD_AFTER_ARG`]
//! (`agent::tinyagents::middleware::shell_yield`, which does so only when the
//! turn can follow a process with `shell_process`) and the turn has room for
//! another process.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tinytools::{ToolResult, ToolRunContext};

use super::shell_support::{exec_result, yield_after, yielded_output, YIELD_AFTER_ARG};
use crate::sandbox::shell_exec::ShellRunOutcome;
use crate::sandbox::shell_processes::{Ended, Slot};

/// Records the call's audit entry: whether the command passed the in-tool
/// checks, whether it succeeded, and how long it ran, in milliseconds.
pub(super) type AuditHook = Box<dyn FnOnce(bool, bool, u64) + Send>;

/// The call's audit entry, written once: right after the call, or, when the
/// command is left running, when the turn's registry sees it end.
pub(super) type AuditOnce = Mutex<Option<AuditHook>>;

/// What a call asked about coming back before its command ends.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct YieldRequest {
    /// The turn lets this call yield: it moved the model's `timeout_secs` to
    /// [`YIELD_AFTER_ARG`].
    pub(super) allowed: bool,
    /// The model's own come-back time, if it gave one.
    pub(super) requested: Option<u64>,
}

impl YieldRequest {
    pub(super) fn from_args(args: &serde_json::Value) -> Self {
        let value = args.get(YIELD_AFTER_ARG);
        Self {
            allowed: value.is_some(),
            requested: value.and_then(serde_json::Value::as_u64),
        }
    }

    /// The deadline for a backend that cannot leave a command running: the
    /// command's own deadline, or the model's come-back time if that is
    /// sooner, so the model's `timeout_secs` still bounds the command there.
    pub(super) fn deadline_instead(&self, deadline: Option<u64>) -> Option<u64> {
        match (
            deadline.filter(|secs| *secs > 0),
            self.requested.filter(|secs| *secs > 0),
        ) {
            (Some(deadline), Some(requested)) => Some(deadline.min(requested)),
            (deadline, requested) => deadline.or(requested),
        }
    }
}

/// A place in the turn's registry for this call, when it may yield.
fn slot(request: YieldRequest, context: Option<&dyn ToolRunContext>) -> Option<Slot> {
    if !request.allowed {
        return None;
    }
    let slot =
        crate::agent::tinyagents::host::run_context::shell_processes_for_tool(context)?.reserve();
    if slot.is_none() {
        tracing::debug!(
            "[shell] the turn is following its limit of processes; waiting for the end"
        );
    }
    slot
}

/// The yield point for this call with a slot reserved for it, and the
/// command's deadline. A call that cannot yield (a backend that runs commands
/// one-shot, no place left in the turn, or a deliberate wait) reserves
/// nothing and keeps the model's come-back time as its deadline, as
/// `timeout_secs` was, so it still comes back by then.
fn come_back(
    request: YieldRequest,
    context: Option<&dyn ToolRunContext>,
    command: &str,
    deadline: Duration,
    backend_can_yield: bool,
) -> (Option<Slot>, Option<Duration>, Duration) {
    let slot = slot(request, context).filter(|_| backend_can_yield);
    let after = slot
        .as_ref()
        .and_then(|_| yield_after(request.requested, deadline, command));
    if after.is_some() {
        return (slot, after, deadline);
    }
    let deadline = request
        .requested
        .filter(|secs| *secs > 0)
        .map_or(deadline, |secs| deadline.min(Duration::from_secs(secs)));
    (None, None, deadline)
}

/// The tool result for a run, adopting a command still running into the
/// turn's registry; its audit entry then waits for the end.
fn finish(
    outcome: anyhow::Result<ShellRunOutcome>,
    slot: Option<Slot>,
    command: &str,
    started: Instant,
    timeout: Duration,
    audit: &AuditOnce,
) -> ToolResult {
    let live = match outcome {
        Ok(ShellRunOutcome::Running(live)) => live,
        Ok(ShellRunOutcome::Finished(result)) => return exec_result(Ok(result), timeout),
        Err(error) => return exec_result(Err(error), timeout),
    };
    let Some(slot) = slot else {
        // Unreachable: no yield point without a slot. The dropped handle
        // stops the process.
        return ToolResult::error("Command was left running unexpectedly.");
    };
    let (stdout, stderr) = (live.stdout.clone(), live.stderr.clone());
    let on_end = audit
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .map(|hook| -> crate::sandbox::shell_processes::OnEnd {
            Box::new(move |ended: Ended, ran: Duration| {
                hook(
                    true,
                    ended == Ended::Exited(0),
                    u64::try_from(ran.as_millis()).unwrap_or(u64::MAX),
                );
            })
        });
    let id = slot.adopt(live, command, started, timeout, on_end);
    ToolResult::success(yielded_output(
        &id,
        started.elapsed(),
        timeout,
        &stdout,
        &stderr,
    ))
}

/// Run a native shell command for a `shell` call.
pub(super) async fn run_native(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &crate::config::Config,
    mut run: crate::sandbox::shell_exec::NativeShellRun,
    request: YieldRequest,
    context: Option<&dyn ToolRunContext>,
    audit: &AuditOnce,
) -> ToolResult {
    let (slot, after, deadline) = come_back(request, context, &run.command, run.timeout, true);
    run.yield_after = after;
    run.timeout = deadline;
    let (command, timeout) = (run.command.clone(), run.timeout);
    let started = Instant::now();
    let outcome = crate::sandbox::shell_exec::run_yielding(ctx, config, run).await;
    finish(outcome, slot, &command, started, timeout, audit)
}

/// Run a command under a resolved sandbox `policy` for a `shell` call.
pub(super) async fn run_in_policy(
    ctx: &crate::core::runtime::DispatchCtx,
    policy: crate::sandbox::SandboxPolicy,
    command: &str,
    requested_timeout: Option<u64>,
    request: YieldRequest,
    action_dir: &std::path::Path,
    context: Option<&dyn ToolRunContext>,
    meter: Option<crate::sandbox::shell_progress::ShellOutputMeter>,
    audit: &AuditOnce,
) -> (bool, ToolResult) {
    tracing::debug!(
        backend = ?policy.backend,
        command = command,
        "[shell] routing to sandbox backend"
    );

    let mut extra_env = std::collections::HashMap::<std::ffi::OsString, std::ffi::OsString>::new();
    let command_environment =
        crate::agent::tinyagents::host::run_context::command_environment_for_tool(context);
    super::shell_support::extend_explicit_environment(
        &mut extra_env,
        command_environment.as_deref(),
    );
    // Apply the same Git config hardening to local and sandboxed shells.
    extra_env.extend(tinytools_std::filesystem::shell_git_env());
    // And the same Python defaults the native path sets.
    for (var, val) in super::shell_support::python_env_defaults(|name| {
        command_environment
            .as_ref()
            .and_then(|overrides| overrides.get(name).map(std::ffi::OsString::from))
            .or_else(|| {
                command_environment
                    .is_none()
                    .then(|| std::env::var_os(name))
                    .flatten()
            })
    }) {
        extra_env.insert(var.into(), val);
    }

    // Sandbox backends require a finite deadline. Without an explicit
    // `timeout_secs`, substitute the generous effective-unbounded cap so a
    // long command isn't killed while still bounding a wedged sandbox.
    let explicit_timeout = tinytools_std::timeout::explicit_call_timeout_duration(
        requested_timeout,
        tinytools_std::timeout::MAX_TIMEOUT_SECS,
    );
    let effective = explicit_timeout
        .unwrap_or_else(|| Duration::from_secs(tinytools_std::timeout::SANDBOX_UNBOUNDED_CAP_SECS));
    // Resolve the backend before reserving: Docker cannot leave a command
    // running, so a Docker run takes no place in the turn and is bounded by
    // the model's time instead (#8317).
    let resolved = match crate::sandbox::ops::resolve_sandbox(ctx, &policy).await {
        Ok(resolved) => resolved,
        Err(error) => return (true, exec_result(Err(error), effective)),
    };
    let (slot, yield_after, effective) =
        come_back(request, context, command, effective, resolved.can_yield());
    tracing::debug!(
        backend = ?resolved.backend(),
        timeout_secs = effective.as_secs(),
        requested_timeout_secs = ?requested_timeout,
        unbounded = explicit_timeout.is_none(),
        yield_after_secs = ?yield_after.map(|after| after.as_secs()),
        "[shell] starting sandboxed command"
    );

    let started = Instant::now();
    let outcome = crate::sandbox::ops::execute_resolved(
        ctx,
        resolved,
        command,
        action_dir,
        extra_env,
        effective,
        yield_after,
        meter,
    )
    .await;
    let result = finish(outcome, slot, command, started, effective, audit);
    (true, result)
}

/// The call's audit entry, to be written once by whoever learns the outcome.
pub(super) fn audit_once(
    audit: Arc<crate::security::AuditLogger>,
    command: String,
    approved: bool,
) -> AuditOnce {
    Mutex::new(Some(Box::new(move |allowed, success, duration_ms| {
        super::shell::record_audit(&audit, &command, approved, allowed, success, duration_ms);
    })))
}

#[cfg(test)]
#[path = "shell_run_tests.rs"]
mod tests;
