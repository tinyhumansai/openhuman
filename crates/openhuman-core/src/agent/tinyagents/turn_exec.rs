//! Drive an openhuman agent turn through the `tinyagents` runtime turn driver.
//!
//! [`TurnSpec::run`] is the entry point the channel and sub-agent routes use and
//! [`TurnSpec::run_hosted`] the one for hosted root turns. Both plan the turn
//! with [`OpenHumanTurnPlanner`], let `tinyagents_runtime::turn_driver::TurnDriver` assemble and wire the harness
//! (models, run policy, tools, steering, early exit, the cap pause), then install
//! the host middleware stack ([`turn_middleware`](super::turn_middleware)) and
//! finalize the run into a [`TinyagentsTurnOutcome`].

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use anyhow::Result;
use tinyagents_harness::runtime::{AgentInvocation, AgentTurnRequest, InvocationRuntime};
use tinyagents_harness::store::{InMemoryStore as ToolResultArtifactIndexStore, StoreRegistry};
use tinyagents_harness::tool::ToolSnapshot;
use tinyagents_harness::turn_policy::effective_max_iterations;
use tinyagents_runtime::turn_driver::{
    SubagentScope as DriverSubagentScope, TurnControls, TurnDriver, TurnScope,
};
use tinyagents_runtime::DriverRequest;

use crate::agent::harness::tool_result_artifacts::TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent::tinyagents::middleware::turn_context::TurnContextMiddleware;
use crate::agent::tinyagents::observability::{OpenhumanEventBridge, SubagentScope};
use crate::agent::tinyagents::tool_registration::capability_diagnostics;
use crate::agent::tinyagents::turn_middleware::{
    install_turn_middleware, TurnMiddlewareInputs, TurnWiring,
};
use crate::agent::tinyagents::turn_outcome::TinyagentsTurnOutcome;
use crate::agent::tinyagents::turn_plan::{
    enforce_registry_diagnostics, turn_models_for_plan, turn_run_policy, OpenHumanTurnPlanner,
};
use crate::agent::tinyagents::turn_run_error::map_turn_run_error;
use crate::agent::tinyagents::turn_run_finalize::finalize_turn_outcome;
use crate::agent::tinyagents::NativeSubagentRunContext;
use crate::agent::tinyagents::{journal, steering_forwarder};
use crate::inference::turn_models::TurnModels;
use tinyagents_harness::events::EventSink;
use tinyagents_harness::observability::turn_observer;
use tinyagents_harness::run_queue::RunQueue;
use tinyagents_harness::turn_outcome::ToolOutcomeSink;
use tinyagents_session::transcript::TranscriptMessage;

use super::ToolPolicyEnforcement;
mod root_hosted_harness;
use root_hosted_harness::root_hosted_harness;

#[cfg(test)]
#[path = "turn_exec_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "budget_stop_turn_tests.rs"]
mod budget_stop_tests;

/// Everything one tinyagents turn needs, named.
///
/// Replaces the two positional entry functions this module used to export (one
/// took twenty arguments, two of them adjacent `bool`s): a call site now names
/// each field, so a reordering is visible in the expression.
///
/// `tool_sets` are the routes' **shared**, `Arc`-owned tool registry sets,
/// consulted in order. `allowed` is the callable tool-name whitelist with
/// **fail-closed** semantics (issue #4452): `None` means "no filter supplied"
/// so every tool visible in `tool_sets` is registered; `Some(set)` registers
/// *exactly* the named tools, so `Some(empty)` is an explicit deny-all. That
/// stops a tool-less sub-agent from inheriting the parent's full tool surface.
///
/// `run_queue` forwards mid-flight steer messages into the run;
/// `subagent_scope` re-scopes progress to the `Subagent*` variants (child
/// runs); `early_exit_tools` name the tools that pause the loop (for example
/// `ask_user_clarification`) and surface the question via
/// [`TinyagentsTurnOutcome::early_exit_tool`]. `max_iterations == 0` means
/// "unset". When `context_window` is known the context middleware keeps history
/// under budget.
///
/// `defer_turn_completed_to_caller` (#4457): when `true`, the turn does not emit
/// the terminal `TurnCompleted`; the caller emits it after its post-run wrap-up
/// (the chat path streams a cap checkpoint first).
pub(crate) struct TurnSpec<'a> {
    pub run_context: OpenHumanRunContext,
    pub turn_models: TurnModels,
    pub provider_id: String,
    pub model: &'a str,
    pub history: Vec<TranscriptMessage>,
    pub tool_sets: Vec<Arc<Vec<Box<dyn tinytools::Tool>>>>,
    pub allowed: Option<HashSet<String>>,
    pub max_iterations: usize,
    pub subagent_scope: Option<SubagentScope>,
    pub context_window: Option<u64>,
    pub run_queue: Option<Arc<RunQueue<crate::agent::queued_turn::QueuedTurn>>>,
    pub message_run_queue: Option<tinyagents_harness::run_queue::RunQueueHandle>,
    pub early_exit_tools: &'a [&'a str],
    pub pause_at_cap: bool,
    pub max_output_tokens: Option<u32>,
    pub context_mw: TurnContextMiddleware,
    pub tool_policy: Option<ToolPolicyEnforcement>,
    pub deterministic_cacheable: bool,
    pub defer_turn_completed_to_caller: bool,
}

impl TurnSpec<'_> {
    /// Plan the turn with [`OpenHumanTurnPlanner`] and drive it on the runtime
    /// `TurnDriver` (channel/CLI and sub-agent routes).
    pub(crate) async fn run(self) -> Result<TinyagentsTurnOutcome> {
        Box::pin(drive::drive_turn(self, None, None)).await
    }

    pub(crate) async fn run_with_native_subagent(
        self,
        native: NativeSubagentRunContext,
    ) -> Result<TinyagentsTurnOutcome> {
        Box::pin(drive::drive_turn(self, None, Some(native))).await
    }

    /// Drive a hosted root turn: the same plan, executed through
    /// `AgentInvocation` with durable host authority attached for each root so
    /// concurrent sessions cannot replace one another's security or workspace
    /// state on a shared harness. A root is never a sub-agent and never serves
    /// a cached response, so those two fields are ignored.
    pub(crate) async fn run_hosted(
        mut self,
        hosted_base: Arc<crate::agent::tinyagents::host::OpenHumanHostBase>,
        agent_id: String,
    ) -> Result<TinyagentsTurnOutcome> {
        self.subagent_scope = None;
        self.deterministic_cacheable = false;
        Box::pin(drive::drive_turn(self, Some((hosted_base, agent_id)), None)).await
    }
}

#[path = "turn_exec/drive.rs"]
mod drive;
