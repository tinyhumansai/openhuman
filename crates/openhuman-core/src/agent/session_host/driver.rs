//! OpenHuman's model-and-harness implementation of the neutral session driver.
//!
//! This adapter holds no transcript, prefix, or generic history; its owning
//! `tinyagents_runtime::Session` supplies the complete immutable history and tool snapshot per call.

mod grounded_close;
mod helpers;
use crate::agent::tinyagents::SupplementalTrace;
use helpers::{
    append_synthesized_block, driver_cancelled_with_snapshot, driver_error,
    driver_error_with_snapshot, driver_failure, ensure_snapshot_tools_are_executable,
};

use std::{collections::HashSet, sync::Arc, time::Instant};

use async_trait::async_trait;
use tinyagents_runtime::{
    DriverFailure, DriverOutcome, DriverRequest, RuntimeError, SessionDriver, TranscriptPartial,
};
use tinyagents_session::transcript::TranscriptMessage;
use tinyinference_llm::message::Message;
use tinytools_agent::dialect::ToolDialect;

use crate::agent::tinyagents::{
    host::OpenHumanHostBase, host::OpenHumanRunContext, TurnModelSource, TurnSpec,
};

/// Immutable host composition consumed by [`OpenHumanSessionDriver`].
///
/// Mutable per-turn policy belongs in `OpenHumanSessionHooks`; the driver only
/// receives its final context, history, and immutable tool declaration view.
pub struct OpenHumanSessionDriver {
    turn_model_source: TurnModelSource,
    dispatcher: Arc<dyn ToolDialect>,
    model_name: String,
    temperature: f64,
    max_iterations: usize,
    model_vision: bool,
    run_queue:
        Option<Arc<tinyagents_harness::run_queue::RunQueue<crate::agent::queued_turn::QueuedTurn>>>,
    workspace: Option<tinytools::WorkspaceDescriptor>,
    sandbox_mode: tinyagents_definition::SandboxMode,
    hosted_base: Option<Arc<OpenHumanHostBase>>,
    agent_id: String,
}

impl OpenHumanSessionDriver {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        turn_model_source: TurnModelSource,
        dispatcher: Arc<dyn ToolDialect>,
        model_name: String,
        temperature: f64,
        max_iterations: usize,
        model_vision: bool,
        run_queue: Option<
            Arc<tinyagents_harness::run_queue::RunQueue<crate::agent::queued_turn::QueuedTurn>>,
        >,
        workspace: Option<tinytools::WorkspaceDescriptor>,
        sandbox_mode: tinyagents_definition::SandboxMode,
        hosted_base: Option<Arc<OpenHumanHostBase>>,
        agent_id: String,
    ) -> Self {
        Self {
            turn_model_source,
            dispatcher,
            model_name,
            temperature,
            max_iterations,
            model_vision,
            run_queue,
            workspace,
            sandbox_mode,
            hosted_base,
            agent_id,
        }
    }
}

#[path = "driver/execute.rs"]
mod execute;

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;
