//! Journal-backed span projection (C4 slice S2).
//!
//! Reconstructs a run's [`AgentProgress`] stream from the durable
//! [`AgentObservation`] journal (the crate `AgentEvent` record) and folds it
//! through the existing [`SpanCollector`], so trace spans no longer require the
//! *live* in-run `AgentProgress` side-observer
//! (`web_chat/progress_bridge.rs`). A UI/supervisor can attach
//! after a run, read the journal, and rebuild identical spans.
//!
//! This is deliberately built on `SpanCollector` (not a re-derivation) so span
//! *shape* parity holds by construction for every `AgentProgress` the journal
//! can produce. The one-way mapping here mirrors `OpenhumanEventBridge`
//! (`tinyagents/observability.rs`) but is **pure** — it depends only on the
//! journalled event, made possible by the crate carrying tool outcome
//! (`duration_ms`/`output_bytes`/`error`) on `ToolCompleted` (tinyagents#18).
//!
//! Known parity gaps (see `docs/.../C4-journal-progress-parity-plan.md` §2a):
//! - `ModelCallCompleted.cost_usd` and `cache_creation_tokens` are not on the
//!   crate event; filled as `0` here and to be sourced from the persisted
//!   per-run cost store at export time.
//! - Sub-agent prompt/output content is not on the crate lifecycle events, so
//!   subagent spans carry lifecycle/timing and child tool/model structure but
//!   empty delegated prompt/final output until a richer journal event exists.

use tinyagents::harness::events::AgentEvent;
use tinyagents::harness::observability::AgentObservation;

use super::{SpanCollector, TraceContext, TraceSpan};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::tool_status::classify;

/// Mutable state threaded across a single run's observations while replaying.
#[derive(Default)]
struct ReplayState {
    /// 1-based iteration index, bumped once per `ModelStarted` — the same
    /// attribution the live `IterationCursor` provides.
    iteration: u32,
    /// Max iterations configured for the turn (carried onto iteration spans).
    max_iterations: u32,
    /// `call_id → model name`, learned from `ModelStarted` so the matching
    /// `ModelCompleted` can name its generation span (the crate `ModelCompleted`
    /// event carries no model name).
    models: std::collections::HashMap<String, String>,
    /// Stack of currently-open sub-agent runs. The crate lifecycle event only
    /// carries name/depth; ordered replay brackets child model/tool events.
    subagents: Vec<ReplaySubagent>,
    /// Monotonic suffix to make repeated invocations of the same child name
    /// distinct in the span tree.
    next_subagent_seq: u64,
}

#[derive(Clone)]
struct ReplaySubagent {
    agent_id: String,
    task_id: String,
    depth: usize,
    iteration: u32,
    started_ts_ms: u64,
}

impl ReplayState {
    fn active_subagent(&self) -> Option<&ReplaySubagent> {
        self.subagents.last()
    }

    fn active_subagent_mut(&mut self) -> Option<&mut ReplaySubagent> {
        self.subagents.last_mut()
    }
}

/// Maps one journalled observation to zero or more [`AgentProgress`] events,
/// updating `state`. Non-span-bearing events (deltas, budget/cache/steering
/// diagnostics) map to nothing — `SpanCollector` ignores them anyway.
fn observation_to_progress(obs: &AgentObservation, state: &mut ReplayState) -> Vec<AgentProgress> {
    let event = &obs.event;
    match event {
        AgentEvent::RunStarted { .. } => {
            if state.active_subagent().is_some() {
                Vec::new()
            } else {
                vec![AgentProgress::TurnStarted]
            }
        }

        AgentEvent::ModelStarted { call_id, model } => {
            let iteration = match state.active_subagent_mut() {
                Some(scope) => {
                    scope.iteration += 1;
                    scope.iteration
                }
                None => {
                    state.iteration += 1;
                    state.iteration
                }
            };
            state
                .models
                .insert(call_id.as_str().to_string(), model.clone());
            match state.active_subagent() {
                Some(scope) => vec![AgentProgress::SubagentIterationStarted {
                    agent_id: scope.agent_id.clone(),
                    task_id: scope.task_id.clone(),
                    iteration,
                    max_iterations: state.max_iterations,
                    extended_policy: false,
                }],
                None => vec![AgentProgress::IterationStarted {
                    iteration,
                    max_iterations: state.max_iterations,
                }],
            }
        }

        AgentEvent::ToolStarted { call_id, tool_name } => match state.active_subagent() {
            Some(scope) => vec![AgentProgress::SubagentToolCallStarted {
                agent_id: scope.agent_id.clone(),
                task_id: scope.task_id.clone(),
                call_id: call_id.as_str().to_string(),
                tool_name: tool_name.clone(),
                arguments: serde_json::Value::Null,
                iteration: scope.iteration,
                display_label: None,
                display_detail: None,
            }],
            None => vec![AgentProgress::ToolCallStarted {
                call_id: call_id.as_str().to_string(),
                tool_name: tool_name.clone(),
                // The journal does not carry the model's raw argument JSON in
                // payload-free mode; the tool span still renders from name + id.
                arguments: serde_json::Value::Null,
                iteration: state.iteration,
                display_label: None,
                display_detail: None,
            }],
        },

        AgentEvent::ToolCompleted {
            call_id,
            tool_name,
            input,
            output,
            duration_ms,
            output_bytes,
            error,
            ..
        } => {
            // Outcome now rides the event (tinyagents#18): success, duration and
            // size are self-describing, and the same `classify` the live path
            // uses reproduces the identical `ClassifiedFailure` from the
            // journalled error string. `output` is present only when the run
            // captured payloads (full-content journals).
            let failure = error.as_ref().map(|text| classify(text, false));
            let output_text = match output {
                Some(serde_json::Value::String(text)) => text.clone(),
                Some(value) => value.to_string(),
                None => String::new(),
            };
            match state.active_subagent() {
                Some(scope) => vec![AgentProgress::SubagentToolCallCompleted {
                    agent_id: scope.agent_id.clone(),
                    task_id: scope.task_id.clone(),
                    call_id: call_id.as_str().to_string(),
                    tool_name: tool_name.clone(),
                    success: error.is_none(),
                    output_chars: output_bytes.unwrap_or(0) as usize,
                    output: output_text,
                    arguments: input.clone(),
                    elapsed_ms: duration_ms.unwrap_or(0),
                    iteration: scope.iteration,
                    failure,
                }],
                None => vec![AgentProgress::ToolCallCompleted {
                    call_id: call_id.as_str().to_string(),
                    tool_name: tool_name.clone(),
                    success: error.is_none(),
                    output_chars: output_bytes.unwrap_or(0) as usize,
                    output: output_text,
                    arguments: input.clone(),
                    elapsed_ms: duration_ms.unwrap_or(0),
                    iteration: state.iteration,
                    failure,
                }],
            }
        }

        AgentEvent::ModelCompleted {
            call_id,
            usage,
            input,
            output,
            ..
        } => {
            let model = state
                .models
                .get(call_id.as_str())
                .cloned()
                .unwrap_or_default();
            let usage = usage.unwrap_or_default();
            let scope = state.active_subagent().cloned();
            let iteration = scope
                .as_ref()
                .map(|s| s.iteration)
                .unwrap_or(state.iteration);
            let mut progress = vec![AgentProgress::ModelCallCompleted {
                model,
                // Provider qualification/cost are filled at export time from the
                // persisted cost store, not the journal (§2a).
                provider_id: String::new(),
                subagent_task_id: scope.as_ref().map(|s| s.task_id.clone()),
                input: input.clone(),
                output: output.clone(),
                iteration,
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cached_input_tokens: usage.cache_read_tokens,
                cache_creation_tokens: 0,
                reasoning_tokens: usage.reasoning_tokens,
                cost_usd: 0.0,
            }];
            if scope.is_none() {
                let turn_input = input.as_ref().map(json_content_text);
                let turn_output = output.as_ref().map(json_content_text);
                if turn_input.is_some() || turn_output.is_some() {
                    progress.push(AgentProgress::TurnContent {
                        input: turn_input,
                        output: turn_output,
                    });
                }
            }
            progress
        }

        AgentEvent::SubAgentStarted { name, depth } => {
            state.next_subagent_seq += 1;
            let task_id = format!("{name}-d{depth}-{}", state.next_subagent_seq);
            state.subagents.push(ReplaySubagent {
                agent_id: name.clone(),
                task_id: task_id.clone(),
                depth: *depth,
                iteration: 0,
                started_ts_ms: obs.ts_ms,
            });
            vec![AgentProgress::SubagentSpawned {
                agent_id: name.clone(),
                task_id,
                mode: "typed".to_string(),
                dedicated_thread: false,
                prompt_chars: 0,
                worker_thread_id: None,
                display_name: Some(name.clone()),
                prompt: String::new(),
            }]
        }

        AgentEvent::SubAgentCompleted { name, depth } => {
            let pos = state
                .subagents
                .iter()
                .rposition(|scope| scope.agent_id == *name && scope.depth == *depth);
            let Some(scope) = pos.map(|index| state.subagents.remove(index)) else {
                return Vec::new();
            };
            vec![AgentProgress::SubagentCompleted {
                agent_id: scope.agent_id,
                task_id: scope.task_id,
                elapsed_ms: obs.ts_ms.saturating_sub(scope.started_ts_ms),
                iterations: scope.iteration,
                output_chars: 0,
                output: String::new(),
                worktree_path: None,
                changed_files: Vec::new(),
                dirty_status: None,
            }]
        }

        AgentEvent::RunCompleted { .. } => {
            if state.active_subagent().is_some() {
                Vec::new()
            } else {
                vec![AgentProgress::TurnCompleted {
                    iterations: state.iteration,
                }]
            }
        }

        AgentEvent::RunFailed { error, .. } => {
            let Some(scope) = state.subagents.pop() else {
                return Vec::new();
            };
            vec![AgentProgress::SubagentFailed {
                agent_id: scope.agent_id,
                task_id: scope.task_id,
                error: error.clone(),
            }]
        }

        _ => Vec::new(),
    }
}

fn json_content_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Object(map) => map
            .get("content")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string()),
        _ => value.to_string(),
    }
}

/// Projects a run's journalled `observations` into trace spans by replaying
/// them into [`AgentProgress`] and folding through a fresh [`SpanCollector`],
/// stamped with each observation's journal timestamp (`ts_ms`).
pub(crate) fn spans_from_observations(
    ctx: TraceContext,
    max_iterations: u32,
    observations: &[AgentObservation],
) -> Vec<TraceSpan> {
    let capture_content = ctx.capture_content;
    let mut collector = SpanCollector::new(ctx).with_content_capture(capture_content);
    let mut state = ReplayState {
        max_iterations,
        ..ReplayState::default()
    };
    let mut last_ts = 0;
    for obs in observations {
        last_ts = obs.ts_ms;
        for progress in observation_to_progress(obs, &mut state) {
            collector.record(&progress, obs.ts_ms);
        }
    }
    collector.finish(last_ts);
    collector.spans().to_vec()
}
