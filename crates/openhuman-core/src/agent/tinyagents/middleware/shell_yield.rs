//! A turn's `shell` call: the model's `timeout_secs` is when the call comes
//! back, not when the command is killed.
//!
//! A command still running at that point keeps running as a process the model
//! follows with `shell_process` (`tools/impl/system/shell.rs`). The kill is the
//! turn's: TinyAgents' `ShellTurnBudget` clamp writes the turn's remaining
//! budget into `timeout_secs`, and the shell enforces that as the command's
//! deadline. So this runs just before the clamp and moves the model's value to
//! the shell's yield argument, leaving `timeout_secs` to the clamp. The yield
//! argument's presence is what lets the call yield, so it is set (to `null`
//! when the model gave no `timeout_secs`) on every shell call of a turn that
//! registers `shell_process`. A turn without it (an agent whose tool belt names
//! `shell` alone) could not follow a process, so nothing moves and
//! `timeout_secs` stays a kill, as it was. Outside a turn nothing moves it
//! either.

use async_trait::async_trait;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::error::Result as TaResult;
use tinyagents_harness::middleware::Middleware;
use tinyinference_llm::tool::ToolCall as TaToolCall;

use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::tools::implementations::system::YIELD_AFTER_ARG;

/// Moves a `shell` call's `timeout_secs` to its yield argument, when the turn
/// can follow a process left running.
pub(crate) struct ShellYieldMiddleware {
    /// The turn registers `shell_process`.
    pub(crate) can_follow: bool,
}

#[async_trait]
impl Middleware<(), OpenHumanRunContext> for ShellYieldMiddleware {
    fn name(&self) -> &str {
        "shell_timeout_yields"
    }

    async fn before_tool(
        &self,
        _ctx: &mut RunContext<OpenHumanRunContext>,
        _state: &(),
        call: &mut TaToolCall,
    ) -> TaResult<()> {
        if call.name == "shell" && self.can_follow {
            if let Some(arguments) = call.arguments.as_object_mut() {
                timeout_to_yield(arguments);
            }
        }
        Ok(())
    }
}

/// Move `timeout_secs` to [`YIELD_AFTER_ARG`], `null` when absent: the call
/// may yield, at its default point unless the model gave one.
pub(crate) fn timeout_to_yield(arguments: &mut serde_json::Map<String, serde_json::Value>) {
    let requested = arguments
        .remove("timeout_secs")
        .unwrap_or(serde_json::Value::Null);
    tracing::debug!(
        requested = %requested,
        "[tinyagents::mw] shell_yield: timeout_secs is the call's yield point"
    );
    arguments.insert(YIELD_AFTER_ARG.to_owned(), requested);
}

#[cfg(test)]
#[path = "shell_yield_tests.rs"]
mod tests;
