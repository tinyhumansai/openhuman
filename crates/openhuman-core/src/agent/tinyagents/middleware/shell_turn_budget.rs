//! Keep a `shell` command's deadline inside the turn's wall-clock budget
//! (#6953).
//!
//! `timeout_secs` accepts up to an hour, and an omitted value runs unbounded,
//! with no regard for how much of the turn is left. When the turn's deadline
//! arrives mid-command the harness kills the whole turn, and the user gets no
//! answer. On Terminal-Bench a model started `timeout 600 …` three minutes
//! before the ceiling and the turn died in it.
//!
//! [`ShellTurnBudget`] clamps the deadline in `before_tool` to what the turn
//! has left minus [`SHELL_TIMEOUT_RESERVE`], so the command ends while there is
//! still time for a model call to answer. The model is told when it happens:
//!
//! - a requested `timeout_secs` that was reduced is always reported
//!   (`[timeout_secs reduced from 660s to 170s: only 260s of the turn budget
//!   remain]`), because the model chose that number;
//! - a deadline set on a command that asked for none is reported only if it
//!   fired. Otherwise it changed nothing the model could see.
//!
//! Two middlewares share the state: [`ShellTimeoutClampMiddleware`] must run
//! after argument recovery so it edits real object arguments, and
//! [`ShellTimeoutNoteMiddleware`] must be registered early so its `after_tool`
//! runs after the output caps and the note survives them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::error::Result as TaResult;
use tinyagents_harness::middleware::{
    Middleware, ToolInvocationIdentity, TurnClock, TurnClockMiddleware,
};
use tinyagents_harness::runtime::AgentHarness;

use crate::agent::tinyagents::host::OpenHumanRunContext;
use tinyinference_llm::tool::ToolCall;
use tinytools::ToolResult;

/// Time kept back from a shell command for the model to read its result and
/// answer.
pub(crate) const SHELL_TIMEOUT_RESERVE: Duration = Duration::from_secs(90);

/// The tool whose deadline this governs.
const SHELL_TOOL: &str = "shell";

/// The shell result prefix when its own `timeout_secs` killed the command.
const SHELL_TIMED_OUT: &str = "Command timed out after";

/// One clamp applied to a shell call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ShellTimeoutClamp {
    /// The model's `timeout_secs`, or `None` when it asked for no deadline.
    pub requested: Option<u64>,
    /// The deadline the command runs under.
    pub clamped_secs: u64,
    /// Turn budget left when the call started.
    pub remaining_secs: u64,
}

impl ShellTimeoutClamp {
    /// The line appended to the call's result.
    pub(crate) fn note(&self) -> String {
        let (clamped, remaining) = (self.clamped_secs, self.remaining_secs);
        match self.requested {
            Some(requested) => format!(
                "[timeout_secs reduced from {requested}s to {clamped}s: only {remaining}s of the turn budget remain]"
            ),
            None => format!(
                "[timeout_secs set to {clamped}s: only {remaining}s of the turn budget remain]"
            ),
        }
    }
}

/// The deadline a shell call may have with `remaining` turn budget left, or
/// `None` when `requested` already fits.
///
/// The cap is what is left minus [`SHELL_TIMEOUT_RESERVE`]. When less than
/// twice the reserve is left, it is half of what is left, so the command and
/// the answer split it. It is never below one second, because `0` means
/// unbounded to the shell. A missing or `0` request is unbounded and always
/// exceeds the cap.
pub(crate) fn clamp_shell_timeout(
    requested: Option<u64>,
    remaining: Duration,
) -> Option<ShellTimeoutClamp> {
    let requested = requested.filter(|secs| *secs > 0);
    let cap = remaining
        .saturating_sub(SHELL_TIMEOUT_RESERVE)
        .max(remaining / 2)
        .as_secs()
        .max(1);
    if requested.is_some_and(|secs| secs <= cap) {
        return None;
    }
    Some(ShellTimeoutClamp {
        requested,
        clamped_secs: cap,
        remaining_secs: remaining.as_secs(),
    })
}

/// Shared state: the turn's budget and the clamps waiting to be reported,
/// keyed by call id.
pub(crate) struct ShellTurnBudget {
    /// The harness policy's wall-clock budget for the turn (see
    /// [`TurnClock::of`]).
    budget: Option<Duration>,
    pending: Mutex<HashMap<String, ShellTimeoutClamp>>,
}

impl ShellTurnBudget {
    pub(crate) fn new(budget: Option<Duration>) -> Arc<Self> {
        Arc::new(Self {
            budget,
            pending: Mutex::default(),
        })
    }

    /// The `before_tool` half, which rewrites the call's `timeout_secs`.
    pub(crate) fn clamp(self: &Arc<Self>) -> ShellTimeoutClampMiddleware {
        ShellTimeoutClampMiddleware(Arc::clone(self))
    }

    /// The `after_tool` half, which reports a clamp on the call's result.
    pub(crate) fn notes(self: &Arc<Self>) -> ShellTimeoutNoteMiddleware {
        ShellTimeoutNoteMiddleware(Arc::clone(self))
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, HashMap<String, ShellTimeoutClamp>> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Clamps a `shell` call's `timeout_secs` to the turn's remainder.
pub(crate) struct ShellTimeoutClampMiddleware(Arc<ShellTurnBudget>);

#[async_trait]
impl<C: Send + Sync> Middleware<(), C> for ShellTimeoutClampMiddleware {
    fn name(&self) -> &str {
        "shell_turn_budget"
    }

    async fn before_tool(
        &self,
        ctx: &mut RunContext<C>,
        _state: &(),
        call: &mut ToolCall,
    ) -> TaResult<()> {
        if call.name != SHELL_TOOL {
            return Ok(());
        }
        let Some(clock) = TurnClock::of(ctx, self.0.budget) else {
            return Ok(());
        };
        let Some(arguments) = call.arguments.as_object_mut() else {
            return Ok(());
        };
        let requested = arguments.get("timeout_secs").and_then(|v| v.as_u64());
        let Some(clamp) = clamp_shell_timeout(requested, clock.remaining()) else {
            return Ok(());
        };
        tracing::debug!(
            run_id = %ctx.run_id(),
            call_id = %call.id,
            requested_secs = ?clamp.requested,
            clamped_secs = clamp.clamped_secs,
            remaining_secs = clamp.remaining_secs,
            "[tinyagents::mw] shell_turn_budget: clamping shell timeout_secs to the turn remainder"
        );
        arguments.insert("timeout_secs".to_string(), clamp.clamped_secs.into());
        self.0.pending().insert(call.id.clone(), clamp);
        Ok(())
    }
}

/// Reports a clamp on the clamped call's result.
pub(crate) struct ShellTimeoutNoteMiddleware(Arc<ShellTurnBudget>);

#[async_trait]
impl<C: Send + Sync> Middleware<(), C> for ShellTimeoutNoteMiddleware {
    fn name(&self) -> &str {
        "shell_turn_budget_note"
    }

    async fn after_tool(
        &self,
        _ctx: &mut RunContext<C>,
        _state: &(),
        invocation: &ToolInvocationIdentity,
        result: &mut ToolResult,
    ) -> TaResult<()> {
        let Some(clamp) = self.0.pending().remove(invocation.call_id().as_str()) else {
            return Ok(());
        };
        let fired = result.is_error && super::tool_result_text(result).starts_with(SHELL_TIMED_OUT);
        if clamp.requested.is_none() && !fired {
            return Ok(());
        }
        tracing::debug!(
            call_id = %invocation.call_id(),
            fired,
            "[tinyagents::mw] shell_turn_budget: reporting the clamped timeout to the model"
        );
        super::append_tool_result_text(result, format!("\n{}", clamp.note()));
        Ok(())
    }
}

/// Install the turn's time notes and return the shell budget whose
/// [`clamp`](ShellTurnBudget::clamp) the caller pushes after argument recovery.
///
/// Pushes [`TurnClockMiddleware`] (`[turn budget: …]` once per tenth of the
/// budget past half) and [`ShellTurnBudget::notes`]. Call it before the
/// repeat-progress guard: `after_tool` runs in reverse registration order, so
/// these notes are appended after every output cap (they survive truncation)
/// and after the guard has fingerprinted the result (a changing note does not
/// make two identical results look different).
pub(crate) fn install_time_notes(
    harness: &mut AgentHarness<(), OpenHumanRunContext>,
) -> Arc<ShellTurnBudget> {
    let budget = harness
        .policy()
        .limits
        .max_wall_clock_ms
        .map(Duration::from_millis);
    tracing::debug!(
        budget_ms = ?budget.map(|b| b.as_millis() as u64),
        "[tinyagents::mw] installing turn clock and shell turn-budget notes"
    );
    harness.push_middleware(Arc::new(TurnClockMiddleware::new(budget)));
    let shell = ShellTurnBudget::new(budget);
    harness.push_middleware(Arc::new(shell.notes()));
    shell
}

#[cfg(test)]
#[path = "shell_turn_budget_tests.rs"]
mod tests;
