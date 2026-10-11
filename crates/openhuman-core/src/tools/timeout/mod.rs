//! Wall-clock timeout for tool execution (node/tool runtime + agent loop).
//!
//! Resolution order, highest precedence first:
//! 1. `OPENHUMAN_TOOL_TIMEOUT_SECS` environment variable (operator override).
//! 2. The value pushed in from the persisted config via [`set_tool_timeout_secs`]
//!    (driven by the UI / `config.update_agent_settings` RPC).
//! 3. The built-in [`DEFAULT_TIMEOUT_SECS`] (120) default.
//!
//! The effective value lives in a process-global vendored
//! [`ToolTimeoutSettings`] (atomic inside) and is read
//! fresh on every tool call, so a UI change takes effect on the **next** tool
//! call without a restart. The operator env var, when set to a valid value,
//! always wins — config pushes are ignored while it is present (logged).

use std::sync::OnceLock;
use std::time::Duration;

use tinyagents_harness::tool::ToolTimeoutSettings;
use tinytools::ToolTimeout;

mod command_environment;
pub use command_environment::CommandEnvironment;
mod group_exit;
mod process_cleanup;
pub use process_cleanup::ProcessCleanup;

/// Default tool-execution timeout in seconds when nothing else is configured.
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// Smallest accepted timeout. `0` would disable the timeout entirely, so it is
/// rejected and falls back to the default.
pub const MIN_TIMEOUT_SECS: u64 = 1;
/// Largest accepted timeout (1 hour) — guards against typos that would make a
/// hung tool wedge a session indefinitely.
pub const MAX_TIMEOUT_SECS: u64 = 3600;
/// Operator override env var. Takes precedence over the persisted config value.
pub const ENV_VAR: &str = "OPENHUMAN_TOOL_TIMEOUT_SECS";
/// Effective-unbounded cap (24h) for sandbox backends, which require a finite
/// deadline. Scripting tools run truly unbounded on the native path, but the
/// sandbox path substitutes this generous cap when no explicit `timeout_secs`
/// was requested — long enough not to kill a legitimate long job, finite
/// enough to eventually reclaim a wedged sandbox process.
pub const SANDBOX_UNBOUNDED_CAP_SECS: u64 = 86_400;

/// Grace slack (ms) the vendored settings add to an explicit per-call budget.
const TOOL_TIMEOUT_GRACE_MS: u64 = TOOL_TIMEOUT_GRACE_SECS * 1000;

/// Build vendored settings with OpenHuman's bounds and grace, inheriting
/// `inherited_secs`.
fn build_settings(inherited_secs: u64) -> ToolTimeoutSettings {
    ToolTimeoutSettings::new(
        inherited_secs.saturating_mul(1000),
        MIN_TIMEOUT_SECS * 1000,
        MAX_TIMEOUT_SECS * 1000,
        TOOL_TIMEOUT_GRACE_MS,
    )
}

/// Process-global settings. Seeded from env/default on first touch; config
/// pushes overwrite the inherited value through the vendored atomic.
static SETTINGS: OnceLock<ToolTimeoutSettings> = OnceLock::new();

fn settings() -> &'static ToolTimeoutSettings {
    SETTINGS.get_or_init(|| {
        build_settings(resolve_effective(
            DEFAULT_TIMEOUT_SECS,
            read_env().as_deref(),
        ))
    })
}

/// Install the process-global per-tool timeout settings on `harness`.
///
/// The harness enforces a tool's [`ToolTimeout`] policy only when settings are
/// installed (`AgentHarness::with_tool_timeout_settings`); without them every
/// `Inherit` tool — MCP calls, `use_skill` dispatches, integration actions —
/// ran until the run's wall-clock budget, so a hung call blocked the turn for
/// ~600s instead of failing at the configured 120s (regressed when the host's
/// own adapter deadline was removed in f33a398faa). The installed value is a
/// clone of the shared settings, so later [`set_tool_timeout_secs`] pushes
/// reach already-assembled harnesses on their next tool call.
///
/// Long-running tools opt out through their own policy rather than here:
/// the scripting tool (`shell`) are `Unbounded` unless the call
/// passes `timeout_secs`, media generation carries its own budget, and
/// `composio_connect` / `browser` size theirs to the approval park they wait
/// on inside `execute`.
pub fn install_harness_tool_timeouts<State: Send + Sync, Ctx: Send + Sync>(
    harness: &mut tinyagents_harness::runtime::AgentHarness<State, Ctx>,
) {
    install_with(harness, settings().clone());
}

fn install_with<State: Send + Sync, Ctx: Send + Sync>(
    harness: &mut tinyagents_harness::runtime::AgentHarness<State, Ctx>,
    settings: ToolTimeoutSettings,
) {
    tracing::debug!(
        inherited_secs = settings.inherited_timeout().map_or(0, |d| d.as_secs()),
        "[tool_timeout] installing per-tool timeout settings on the harness"
    );
    harness.with_tool_timeout_settings(settings);
}

/// Parse a raw env-var value into a bounded timeout.
///
/// Testable split from the global resolution: this function is pure and never
/// touches global state, so unit tests can exercise every path without racing
/// on the atomic or mutating the process environment.
///
/// - `None` or a non-numeric string returns [`DEFAULT_TIMEOUT_SECS`].
/// - Values outside `MIN_TIMEOUT_SECS..=MAX_TIMEOUT_SECS` are rejected (returns
///   [`DEFAULT_TIMEOUT_SECS`]).
/// - Valid values pass through unchanged.
pub fn parse_tool_timeout_secs(raw: Option<&str>) -> u64 {
    raw.and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| (MIN_TIMEOUT_SECS..=MAX_TIMEOUT_SECS).contains(&n))
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
}

/// The operator env override, if `ENV_VAR` is set to a value inside the valid
/// range. A present-but-invalid env value (non-numeric, `0`, out of range) is
/// treated as "no override" so the config value still applies.
fn env_override_from(raw: Option<&str>) -> Option<u64> {
    raw.and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| (MIN_TIMEOUT_SECS..=MAX_TIMEOUT_SECS).contains(&n))
}

/// Pure resolver used by both seeding and config pushes. Env override wins;
/// otherwise the (bounded) config value applies.
fn resolve_effective(config_secs: u64, env_raw: Option<&str>) -> u64 {
    match env_override_from(env_raw) {
        Some(env) => env,
        None => parse_tool_timeout_secs(Some(&config_secs.to_string())),
    }
}

fn read_env() -> Option<String> {
    std::env::var(ENV_VAR).ok()
}

/// `true` when the operator env var is set to a valid override, meaning UI /
/// config changes to the timeout are ignored in favour of it. Surfaced to the
/// frontend so the settings panel can explain why its control has no effect.
pub fn env_override_active() -> bool {
    env_override_from(read_env().as_deref()).is_some()
}

/// Effective inherited timeout in whole seconds, seeding the global settings
/// from env/default on first read.
fn current_secs() -> u64 {
    settings().inherited_timeout().map_or(0, |d| d.as_secs())
}

/// Push a config-sourced timeout into the runtime. The operator env override,
/// when active, always wins and `config_secs` is ignored (logged at debug).
/// Returns the effective value stored after the call. Idempotent and safe to
/// call repeatedly (e.g. at startup and on every config update).
pub fn set_tool_timeout_secs(config_secs: u64) -> u64 {
    let env_raw = read_env();
    let effective = resolve_effective(config_secs, env_raw.as_deref());
    settings().set_inherited_timeout_ms(effective.saturating_mul(1000));
    if env_override_from(env_raw.as_deref()).is_some() {
        log::debug!(
            "[tool_timeout] config update ignored: env {ENV_VAR}={effective}s overrides requested {config_secs}s"
        );
    } else {
        log::debug!(
            "[tool_timeout] runtime timeout set to {effective}s (requested {config_secs}s)"
        );
    }
    effective
}

/// Effective timeout in seconds — used for logging and matching frontend
/// timeouts. Read fresh on every call.
pub fn tool_execution_timeout_secs() -> u64 {
    current_secs()
}

/// Resolve an **explicit** per-call timeout request for a tool that is
/// otherwise unbounded (the scripting tool: `shell`).
///
/// Unlike most tools — which inherit the global config-driven timeout so a hung
/// network/MCP call can't wedge a session — scripting tools run with **no**
/// default deadline: a build / solver / test run legitimately takes minutes and
/// must not be hard-killed by a default cap (issue #4023). A deadline applies
/// only when the caller explicitly asks for one via `timeout_secs`.
///
/// Returns:
/// - `None` → run unbounded. Used when no `timeout_secs` was supplied
///   (`None`) or it was explicitly disabled (`Some(0)`).
/// - `Some(secs)` → enforce this budget, clamped to `MIN_TIMEOUT_SECS..=cap`.
///   `cap` lets callers with a tighter own-ceiling (e.g. node/npm at 1800s)
///   pass it; most callers pass [`MAX_TIMEOUT_SECS`].
pub fn explicit_call_timeout_secs(requested: Option<u64>, cap: u64) -> Option<u64> {
    let cap = cap.clamp(MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS);
    match requested {
        None | Some(0) => None,
        Some(n) => Some(n.clamp(MIN_TIMEOUT_SECS, cap)),
    }
}

/// [`explicit_call_timeout_secs`] as a [`Duration`], or `None` for unbounded.
pub fn explicit_call_timeout_duration(requested: Option<u64>, cap: u64) -> Option<Duration> {
    explicit_call_timeout_secs(requested, cap).map(Duration::from_secs)
}

/// Extra slack added on top of an explicit per-call budget before the hard
/// `tokio::time::timeout` fires, so a tool that finishes right at its requested
/// deadline isn't killed by scheduler jitter. The user-facing `timeout_secs`
/// reported on a timeout is the un-padded request.
const TOOL_TIMEOUT_GRACE_SECS: u64 = 5;

/// Pure core of [`resolve_tool_deadline`]: the inherited timeout is a parameter
/// so tests can table-drive it without touching the process-global.
/// Run `cmd` to completion with its output captured, or kill it -- and every
/// process it started -- when `deadline` passes.
///
/// `tokio::time::timeout(deadline, cmd.output())` only abandons the future.
/// Without `kill_on_drop` the child is not even signalled, and a shell's
/// pipeline (`grep -r … | head`) is a set of grandchildren that no kill of the
/// direct child reaches anyway. One `grep -rl … /` the shell tool had reported
/// as "timed out after 600s and was killed" ran on for half an hour at a full
/// core inside a two-CPU container, starving the agent that had started it.
///
/// So the child is spawned as the leader of its own process group and the
/// whole group is signalled when the deadline fires. The result has the same
/// shape as `timeout(deadline, cmd.output())`, so a caller's match arms do not
/// change. Stdio is set the way `output()` sets it: stdin closed, both output
/// streams captured.
pub async fn output_or_kill(
    cmd: &mut tokio::process::Command,
    deadline: Duration,
) -> Result<std::io::Result<std::process::Output>, tokio::time::error::Elapsed> {
    tokio::time::timeout(deadline, output_unbounded(cmd)).await
}

/// Capture a command with no deadline, killing its process group if the
/// future is dropped. The child is reaped by an owned waiter even when the
/// caller cancels; a scoped [`ProcessCleanup`] can await that waiter.
pub async fn output_unbounded(
    cmd: &mut tokio::process::Command,
) -> std::io::Result<std::process::Output> {
    output_with_input(cmd, None).await
}

/// Capture a command, optionally supplying stdin, in the current cleanup scope.
pub async fn output_with_input(
    cmd: &mut tokio::process::Command,
    input: Option<Vec<u8>>,
) -> std::io::Result<std::process::Output> {
    use std::process::Stdio;
    cmd.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
    CommandEnvironment::apply(cmd);
    own_process_group(cmd.as_std_mut());
    let mut child = cmd.spawn()?;
    let stdin = child.stdin.take();
    let reaped = process_cleanup::Reaped::register();
    let (cancel, cancellation) = tokio::sync::watch::channel(false);
    let waiter = crate::core::runtime::spawn_scoped(async move {
        let _reaped = reaped;
        let write_input = async move {
            if let (Some(mut stdin), Some(input)) = (stdin, input) {
                use tokio::io::AsyncWriteExt;
                // An early child exit closes stdin. Its exit status/output is
                // authoritative; a broken pipe must not hide it.
                let _ = stdin.write_all(&input).await;
            }
        };
        let (_, output) = tokio::join!(write_input, collect_command_output(child, cancellation));
        output
    });
    let _cancel_on_drop = CancelOnDrop(cancel);
    waiter.await.map_err(std::io::Error::other)?
}

// The caller signals only the owned waiter. It never retains a PID after
// that waiter reaps the child, so late future drops cannot kill a reused PID.
struct CancelOnDrop(tokio::sync::watch::Sender<bool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}

struct CommandGroup(Option<u32>);

impl CommandGroup {
    fn kill(&self) {
        if let Some(pid) = self.0 {
            kill_process_group(pid);
        }
    }
}

impl Drop for CommandGroup {
    fn drop(&mut self) {
        self.kill();
    }
}

async fn collect_command_output(
    mut child: tokio::process::Child,
    mut cancellation: tokio::sync::watch::Receiver<bool>,
) -> std::io::Result<std::process::Output> {
    use tokio::io::AsyncReadExt;

    let mut group = CommandGroup(child.id());
    let mut stdout = child.stdout.take().expect("command stdout is piped");
    let mut stderr = child.stderr.take().expect("command stderr is piped");
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    {
        let drain = async {
            tokio::try_join!(
                stdout.read_to_end(&mut stdout_bytes),
                stderr.read_to_end(&mut stderr_bytes),
            )
        };
        tokio::pin!(drain);
        tokio::select! {
            biased;
            _ = async { let _ = cancellation.wait_for(|cancelled| *cancelled).await; } => {
                // The leader has not been reaped, even if it already exited.
                // Its PID cannot be reused while signalling this group.
                group.kill();
                child.start_kill()?;
                if let Ok(result) = tokio::time::timeout(Duration::from_secs(2), drain).await {
                    result?;
                }
            }
            result = &mut drain => { result?; }
        }
    }
    let status = tokio::select! {
        biased;
        _ = async { let _ = cancellation.wait_for(|cancelled| *cancelled).await; } => {
            group.kill();
            child.start_kill()?;
            // Pipe EOF and the shell's exit can precede a descendant's exit.
            // Keep the leader unreaped while checking the reserved group.
            let stopped = match group.0 {
                Some(pid) => group_exit::wait(pid).await,
                None => Ok(()),
            };
            let status = child.wait().await?;
            // Disarm before propagating a check error after reaping: its PID
            // could now be reused, including while unwinding this function.
            group.0 = None;
            stopped?;
            status
        }
        result = child.wait() => result?,
    };
    // No await between reaping and disarming. Only this waiter owns the PID.
    group.0 = None;
    Ok(std::process::Output {
        status,
        stdout: stdout_bytes,
        stderr: stderr_bytes,
    })
}

/// Make `cmd` the leader of a new process group when it is spawned, so that
/// [`kill_process_group`] can reach everything it starts. A no-op off Unix.
pub fn own_process_group(cmd: &mut std::process::Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(not(unix))]
    {
        let _ = cmd;
    }
}

/// Send SIGKILL to the process group led by `pid` -- a child spawned via
/// [`own_process_group`] and everything it started. Off Unix the direct child
/// is what `kill_on_drop` reaches and no group exists to signal.
pub fn kill_process_group(pid: u32) {
    #[cfg(unix)]
    {
        let Ok(pid) = i32::try_from(pid) else {
            return;
        };
        // SAFETY: a signal to a process group this process created; the kernel
        // validates the target, and a negative pid addresses the group.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
}

#[cfg(test)]
fn resolve_tool_deadline_with(policy: ToolTimeout, inherited_secs: u64) -> (Option<Duration>, u64) {
    resolve_with(&build_settings(inherited_secs), policy)
}

/// Delegate to the vendored resolver. An explicit millisecond request is
/// rounded up to whole seconds first (OpenHuman reports whole-second budgets);
/// clamping and the grace pad come from [`ToolTimeoutSettings`].
fn resolve_with(settings: &ToolTimeoutSettings, policy: ToolTimeout) -> (Option<Duration>, u64) {
    let policy = match policy {
        ToolTimeout::Millis(req) => ToolTimeout::Millis(
            req.saturating_add(999)
                .saturating_div(1000)
                .saturating_mul(1000),
        ),
        other => other,
    };
    let resolved = settings.resolve(policy);
    (resolved.deadline, resolved.budget_ms / 1000)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
