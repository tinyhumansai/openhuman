//! Progress reporting seam + the shared streaming-delta forwarder.
//!
//! The engine never names a concrete [`AgentProgress`] variant. It talks to a
//! [`ProgressReporter`], whose impls pick the event *flavor*:
//!
//! * [`TurnProgress`] — top-level chat (channel loop, `Agent::turn`): emits the
//!   `Turn*` / `ToolCall*` / `TurnCostUpdated` events and streams provider
//!   deltas as `TextDelta` / `ThinkingDelta` / `ToolCallArgsDelta`.
//! * [`SubagentProgress`] — a spawned sub-agent: emits the `Subagent*` /
//!   `SubagentToolCall*` events (nested under the subagent row in the UI) and
//!   does not stream deltas. The `SubagentSpawned` / `SubagentCompleted` /
//!   `SubagentFailed` lifecycle events stay in the spawn tool, outside the loop.
//! * [`NullProgress`] — triage / tests: every method is a no-op.

use async_trait::async_trait;

use crate::openhuman::agent::cost::TurnCost;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::inference::provider::ProviderDelta;

/// What the engine emits as a turn progresses. All methods default to no-ops so
/// an impl only overrides the events its flavor cares about.
#[async_trait]
pub(crate) trait ProgressReporter: Send + Sync {
    async fn turn_started(&self) {}
    async fn iteration_started(&self, _iteration: u32, _max_iterations: u32) {}
    async fn cost_updated(&self, _model: &str, _iteration: u32, _cost: &TurnCost) {}
    async fn turn_completed(&self, _iterations: u32) {}
    /// `display_label` / `display_detail` carry the server-computed human
    /// label (e.g. "Reading messages") and contextual detail (e.g.
    /// "steven@gmail.com"); `None` lets the client formatter decide.
    #[allow(clippy::too_many_arguments)]
    async fn tool_started(
        &self,
        _call_id: &str,
        _tool_name: &str,
        _arguments: &serde_json::Value,
        _iteration: u32,
        _display_label: Option<&str>,
        _display_detail: Option<&str>,
    ) {
    }
    #[allow(clippy::too_many_arguments)]
    async fn tool_completed(
        &self,
        _call_id: &str,
        _tool_name: &str,
        _success: bool,
        _output: &str,
        _elapsed_ms: u64,
        _iteration: u32,
    ) {
    }

    /// Build the per-iteration `ProviderDelta` streaming sink + forwarder task,
    /// or `(None, None)` when this flavor doesn't stream. Default: no streaming.
    fn make_stream_sink(
        &self,
        _iteration: u32,
    ) -> (
        Option<tokio::sync::mpsc::Sender<ProviderDelta>>,
        Option<tokio::task::JoinHandle<()>>,
    ) {
        (None, None)
    }
}

/// Top-level chat flavor: `Turn*` lifecycle + `ToolCall*` + streaming.
pub(crate) struct TurnProgress {
    pub sink: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
}

impl TurnProgress {
    pub(crate) fn new(sink: Option<tokio::sync::mpsc::Sender<AgentProgress>>) -> Self {
        Self { sink }
    }
}

/// Emit a lifecycle progress event **without ever blocking** the agent's
/// control flow. Progress is pure observability, but the sink is a bounded
/// channel shared by the orchestrator, every inline sub-agent run, and their
/// delta forwarders. A blocking `send().await` here can park a sub-agent's main
/// loop when the channel is momentarily full — which hangs the parent turn,
/// because the orchestrator is `await`ing that sub-agent's tool call and never
/// makes its next LLM request (the subagent-stall flake). So drop the event on
/// `Full` (a missed UI tick, not a correctness issue) and `trace` on `Closed`
/// (no listener). Streaming *text deltas* keep their own blocking backpressure
/// in their forwarder tasks, so visible message text is unaffected.
fn emit(sink: &tokio::sync::mpsc::Sender<AgentProgress>, event: AgentProgress) {
    use tokio::sync::mpsc::error::TrySendError;
    match sink.try_send(event) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            log::trace!("[agent_loop] progress channel full — dropping lifecycle event");
        }
        Err(TrySendError::Closed(_)) => {
            log::trace!("[agent_loop] progress sink closed — dropping lifecycle event");
        }
    }
}

#[async_trait]
impl ProgressReporter for TurnProgress {
    async fn turn_started(&self) {
        if let Some(ref sink) = self.sink {
            emit(sink, AgentProgress::TurnStarted);
        }
    }

    async fn iteration_started(&self, iteration: u32, max_iterations: u32) {
        if let Some(ref sink) = self.sink {
            emit(
                sink,
                AgentProgress::IterationStarted {
                    iteration,
                    max_iterations,
                },
            );
        }
    }

    async fn cost_updated(&self, model: &str, iteration: u32, cost: &TurnCost) {
        if let Some(ref sink) = self.sink {
            let event = AgentProgress::TurnCostUpdated {
                model: model.to_string(),
                iteration,
                input_tokens: cost.input_tokens,
                output_tokens: cost.output_tokens,
                cached_input_tokens: cost.cached_input_tokens,
                total_usd: cost.total_usd(),
            };
            emit(sink, event);
        }
    }

    async fn turn_completed(&self, iterations: u32) {
        if let Some(ref sink) = self.sink {
            emit(sink, AgentProgress::TurnCompleted { iterations });
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn tool_started(
        &self,
        call_id: &str,
        tool_name: &str,
        arguments: &serde_json::Value,
        iteration: u32,
        display_label: Option<&str>,
        display_detail: Option<&str>,
    ) {
        if let Some(ref sink) = self.sink {
            emit(
                sink,
                AgentProgress::ToolCallStarted {
                    call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    arguments: arguments.clone(),
                    iteration,
                    display_label: display_label.map(str::to_string),
                    display_detail: display_detail.map(str::to_string),
                },
            );
        }
    }

    async fn tool_completed(
        &self,
        call_id: &str,
        tool_name: &str,
        success: bool,
        output: &str,
        elapsed_ms: u64,
        iteration: u32,
    ) {
        if let Some(ref sink) = self.sink {
            emit(
                sink,
                AgentProgress::ToolCallCompleted {
                    call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    success,
                    output_chars: output.chars().count(),
                    elapsed_ms,
                    iteration,
                },
            );
        }
    }

    fn make_stream_sink(
        &self,
        iteration: u32,
    ) -> (
        Option<tokio::sync::mpsc::Sender<ProviderDelta>>,
        Option<tokio::task::JoinHandle<()>>,
    ) {
        spawn_delta_forwarder(self.sink.clone(), iteration)
    }
}

/// Sub-agent flavor: `Subagent*` lifecycle + `SubagentToolCall*`, no streaming.
pub(crate) struct SubagentProgress {
    pub sink: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    pub agent_id: String,
    pub task_id: String,
    pub extended_policy: bool,
}

#[async_trait]
impl ProgressReporter for SubagentProgress {
    async fn iteration_started(&self, iteration: u32, max_iterations: u32) {
        if let Some(ref sink) = self.sink {
            emit(
                sink,
                AgentProgress::SubagentIterationStarted {
                    agent_id: self.agent_id.clone(),
                    task_id: self.task_id.clone(),
                    iteration,
                    max_iterations,
                    extended_policy: self.extended_policy,
                },
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn tool_started(
        &self,
        call_id: &str,
        tool_name: &str,
        arguments: &serde_json::Value,
        iteration: u32,
        display_label: Option<&str>,
        display_detail: Option<&str>,
    ) {
        if let Some(ref sink) = self.sink {
            emit(
                sink,
                AgentProgress::SubagentToolCallStarted {
                    agent_id: self.agent_id.clone(),
                    task_id: self.task_id.clone(),
                    call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    arguments: arguments.clone(),
                    iteration,
                    display_label: display_label.map(str::to_string),
                    display_detail: display_detail.map(str::to_string),
                },
            );
        }
    }

    async fn tool_completed(
        &self,
        call_id: &str,
        tool_name: &str,
        success: bool,
        output: &str,
        elapsed_ms: u64,
        iteration: u32,
    ) {
        if let Some(ref sink) = self.sink {
            emit(
                sink,
                AgentProgress::SubagentToolCallCompleted {
                    agent_id: self.agent_id.clone(),
                    task_id: self.task_id.clone(),
                    call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    success,
                    output_chars: output.chars().count(),
                    output: output.to_string(),
                    elapsed_ms,
                    iteration,
                },
            );
        }
    }

    /// Stream the child's visible text + reasoning deltas to the parent,
    /// attributed to this sub-agent's `task_id` so the UI renders them inside
    /// the live subagent row (PR #3007). Tool-call arg fragments are dropped
    /// here — they're already surfaced via the `SubagentToolCall*` lifecycle
    /// events, so forwarding them too would double-render.
    fn make_stream_sink(
        &self,
        iteration: u32,
    ) -> (
        Option<tokio::sync::mpsc::Sender<ProviderDelta>>,
        Option<tokio::task::JoinHandle<()>>,
    ) {
        let Some(sink) = self.sink.clone() else {
            return (None, None);
        };
        let agent_id = self.agent_id.clone();
        let task_id = self.task_id.clone();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<ProviderDelta>(128);
        let forwarder = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let mapped = match event {
                    ProviderDelta::TextDelta { delta } => AgentProgress::SubagentTextDelta {
                        agent_id: agent_id.clone(),
                        task_id: task_id.clone(),
                        delta,
                        iteration,
                    },
                    ProviderDelta::ThinkingDelta { delta } => {
                        AgentProgress::SubagentThinkingDelta {
                            agent_id: agent_id.clone(),
                            task_id: task_id.clone(),
                            delta,
                            iteration,
                        }
                    }
                    ProviderDelta::ToolCallStart { .. }
                    | ProviderDelta::ToolCallArgsDelta { .. } => continue,
                };
                // Await backpressure so streamed deltas arrive in order.
                if sink.send(mapped).await.is_err() {
                    break;
                }
            }
        });
        (Some(tx), Some(forwarder))
    }
}

/// No-op reporter for triage / tests.
pub(crate) struct NullProgress;

impl ProgressReporter for NullProgress {}

/// Spawn a task that forwards `ProviderDelta`s from the provider's streaming
/// channel into `on_progress` as `AgentProgress` delta events, tagged with
/// `iteration` (1-based). Returns the sender to hand to
/// [`crate::openhuman::inference::provider::ChatRequest::stream`] and the task
/// handle to await after the chat call.
///
/// Returns `(None, None)` when there is no progress sink — the caller then
/// passes `stream: None` and the provider uses its non-streaming HTTP path.
///
/// Backpressure discipline: the forwarder `.await`s each `send`, so streamed
/// deltas arrive in order and are never silently dropped when the downstream
/// bridge is slow. It exits cleanly once the sender is dropped (after the chat
/// call) or the downstream closes.
pub(crate) fn spawn_delta_forwarder(
    on_progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    iteration: u32,
) -> (
    Option<tokio::sync::mpsc::Sender<ProviderDelta>>,
    Option<tokio::task::JoinHandle<()>>,
) {
    let Some(progress_sink) = on_progress else {
        return (None, None);
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ProviderDelta>(128);
    let forwarder = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let mapped = match event {
                ProviderDelta::TextDelta { delta } => AgentProgress::TextDelta { delta, iteration },
                ProviderDelta::ThinkingDelta { delta } => {
                    AgentProgress::ThinkingDelta { delta, iteration }
                }
                ProviderDelta::ToolCallStart { call_id, tool_name } => {
                    AgentProgress::ToolCallArgsDelta {
                        call_id,
                        tool_name,
                        delta: String::new(),
                        iteration,
                    }
                }
                ProviderDelta::ToolCallArgsDelta { call_id, delta } => {
                    AgentProgress::ToolCallArgsDelta {
                        call_id,
                        tool_name: String::new(),
                        delta,
                        iteration,
                    }
                }
            };
            if progress_sink.send(mapped).await.is_err() {
                break;
            }
        }
    });
    (Some(tx), Some(forwarder))
}
