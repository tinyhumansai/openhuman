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
    async fn tool_started(
        &self,
        _call_id: &str,
        _tool_name: &str,
        _arguments: &serde_json::Value,
        _iteration: u32,
    ) {
    }
    #[allow(clippy::too_many_arguments)]
    async fn tool_completed(
        &self,
        _call_id: &str,
        _tool_name: &str,
        _success: bool,
        _output_chars: usize,
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

#[async_trait]
impl ProgressReporter for TurnProgress {
    async fn turn_started(&self) {
        if let Some(ref sink) = self.sink {
            if let Err(e) = sink.send(AgentProgress::TurnStarted).await {
                log::warn!("[agent_loop] progress sink closed at TurnStarted: {e}");
            }
        }
    }

    async fn iteration_started(&self, iteration: u32, max_iterations: u32) {
        if let Some(ref sink) = self.sink {
            if let Err(e) = sink
                .send(AgentProgress::IterationStarted {
                    iteration,
                    max_iterations,
                })
                .await
            {
                log::warn!("[agent_loop] progress sink closed at IterationStarted: {e}");
            }
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
            if let Err(e) = sink.send(event).await {
                log::warn!("[agent_loop] progress sink closed at TurnCostUpdated: {e}");
            }
        }
    }

    async fn turn_completed(&self, iterations: u32) {
        if let Some(ref sink) = self.sink {
            if let Err(e) = sink.send(AgentProgress::TurnCompleted { iterations }).await {
                log::warn!("[agent_loop] progress sink closed at TurnCompleted: {e}");
            }
        }
    }

    async fn tool_started(
        &self,
        call_id: &str,
        tool_name: &str,
        arguments: &serde_json::Value,
        iteration: u32,
    ) {
        if let Some(ref sink) = self.sink {
            if let Err(e) = sink
                .send(AgentProgress::ToolCallStarted {
                    call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    arguments: arguments.clone(),
                    iteration,
                })
                .await
            {
                log::warn!("[agent_loop] progress sink closed while emitting ToolCallStarted: {e}");
            }
        }
    }

    async fn tool_completed(
        &self,
        call_id: &str,
        tool_name: &str,
        success: bool,
        output_chars: usize,
        elapsed_ms: u64,
        iteration: u32,
    ) {
        if let Some(ref sink) = self.sink {
            if let Err(e) = sink
                .send(AgentProgress::ToolCallCompleted {
                    call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    success,
                    output_chars,
                    elapsed_ms,
                    iteration,
                })
                .await
            {
                log::warn!(
                    "[agent_loop] progress sink closed while emitting ToolCallCompleted: {e}"
                );
            }
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
}

#[async_trait]
impl ProgressReporter for SubagentProgress {
    async fn iteration_started(&self, iteration: u32, max_iterations: u32) {
        if let Some(ref sink) = self.sink {
            let _ = sink
                .send(AgentProgress::SubagentIterationStarted {
                    agent_id: self.agent_id.clone(),
                    task_id: self.task_id.clone(),
                    iteration,
                    max_iterations,
                })
                .await;
        }
    }

    async fn tool_started(
        &self,
        call_id: &str,
        tool_name: &str,
        _arguments: &serde_json::Value,
        iteration: u32,
    ) {
        if let Some(ref sink) = self.sink {
            let _ = sink
                .send(AgentProgress::SubagentToolCallStarted {
                    agent_id: self.agent_id.clone(),
                    task_id: self.task_id.clone(),
                    call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    iteration,
                })
                .await;
        }
    }

    async fn tool_completed(
        &self,
        call_id: &str,
        tool_name: &str,
        success: bool,
        output_chars: usize,
        elapsed_ms: u64,
        iteration: u32,
    ) {
        if let Some(ref sink) = self.sink {
            let _ = sink
                .send(AgentProgress::SubagentToolCallCompleted {
                    agent_id: self.agent_id.clone(),
                    task_id: self.task_id.clone(),
                    call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    success,
                    output_chars,
                    elapsed_ms,
                    iteration,
                })
                .await;
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
