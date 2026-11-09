//! Host-owned AgentRun lifecycle and cursor event projection.

use std::sync::Arc;

use serde_json::{json, Value};
use tinyflows_bus::runs;

use super::super::agent::{AgentRunState, OpenHumanAgentRunner};
use super::super::binding::CapsHostBinding;

pub(crate) async fn run_agent(
    binding: Arc<CapsHostBinding>,
    request: runs::AgentRunRequest,
    state: Arc<tokio::sync::Mutex<AgentRunState>>,
    cancellation: tokio_util::sync::CancellationToken,
) -> Result<runs::AgentRunOutcome, String> {
    if cancellation.is_cancelled() {
        let mut state = state.lock().await;
        state.error = Some("agent run was cancelled".to_owned());
        state.done = true;
        return Err("agent run was cancelled".to_owned());
    }
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(128);
    let progress_state = Arc::clone(&state);
    let progress = tokio::spawn(async move {
        while let Some(event) = progress_rx.recv().await {
            if let crate::progress::AgentProgress::TurnCompleted {
                stop: Some(stop), ..
            } = &event
            {
                progress_state.lock().await.turn_stop = Some(stop.clone());
            }
            let Some((kind, text)) = progress_event(event) else {
                continue;
            };
            let mut state = progress_state.lock().await;
            if state.events.len() >= runs::MAX_AGENT_TRANSCRIPT_EVENTS {
                state.error = Some("agent progress event limit exceeded".to_owned());
                break;
            }
            let seq = u64::try_from(state.events.len() + 1).unwrap_or(u64::MAX);
            state.events.push(runs::AgentProgressEvent {
                seq,
                at_ms: epoch_millis(),
                kind,
                text: bounded_event_text(text),
            });
        }
    });

    let mut config = request.config.clone();
    if let Some(object) = config.as_object_mut() {
        if object.get("timeout_secs").and_then(Value::as_u64).is_none() {
            if let Some(timeout) = request.agent.limits.agent_timeout_secs {
                object.insert("timeout_secs".to_owned(), Value::from(timeout));
            }
        }
    }
    if let Some(model) = request
        .model
        .model
        .as_ref()
        .or(request.agent.model.as_ref())
    {
        if let Some(object) = config.as_object_mut() {
            if object.get("model").and_then(Value::as_str).is_none() {
                object.insert("model".to_owned(), Value::String(model.clone()));
            }
        }
    }
    let agent_ref = request.agent.id.clone();
    let connection = request.connection_ref.clone();
    let runner = OpenHumanAgentRunner {
        config: Arc::clone(&binding.config),
        ctx: binding.ctx.clone(),
        approval: binding.approval.clone(),
        progress: Some(progress_tx),
    };
    let scope = Arc::clone(&binding);
    let turn = scope.within(async move {
        runner
            .run_agent(&agent_ref, config, connection.as_deref())
            .await
    });
    let result = tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err("agent run was cancelled".to_owned()),
        result = turn => result.map_err(|error| error.to_string()),
    };
    let _ = progress.await;

    let mut state = state.lock().await;
    state.done = true;
    match result {
        Ok(raw) => {
            let turn_stop = state.turn_stop.clone();
            let stop = match turn_stop.as_ref() {
                None => runs::StopReason::Finished,
                Some(stop)
                    if stop.kind == tinyagents_harness::turn_stop::TurnStopKind::IterationCap =>
                {
                    runs::StopReason::LimitStop {
                        limit: "max_steps".to_owned(),
                    }
                }
                Some(stop)
                    if stop.kind == tinyagents_harness::turn_stop::TurnStopKind::WindDown =>
                {
                    runs::StopReason::Paused {
                        token: None,
                        reason: "wall_clock".to_owned(),
                        payload: json!({
                            "kind": stop.kind.as_str(),
                            "failure_class": stop.failure_class.as_ref().map(|class| class.as_str()),
                            "operation": stop.operation.as_deref(),
                        }),
                    }
                }
                Some(stop) => {
                    let class = stop
                        .failure_class
                        .as_ref()
                        .map(|class| class.as_str())
                        .unwrap_or("unknown");
                    let operation = stop.operation.as_deref().unwrap_or("unknown");
                    let error =
                        format!("agent run stopped by classified failure ({class}, {operation})");
                    state.error = Some(error.clone());
                    return Err(error);
                }
            };
            let text = raw
                .as_str()
                .map(str::to_owned)
                .or_else(|| raw.get("text").and_then(Value::as_str).map(str::to_owned));
            let json = matches!(raw, Value::Object(_) | Value::Array(_))
                .then(|| raw.clone())
                .unwrap_or(Value::Null);
            let transcript = state
                .events
                .iter()
                .map(|event| runs::TranscriptEntry {
                    at_ms: event.at_ms,
                    kind: event.kind.clone(),
                    text: event.text.clone(),
                })
                .collect();
            state.outcome = Some(runs::AgentRunOutcome {
                stop,
                text,
                json,
                raw,
                usage: None,
                transcript,
            });
            Ok(state.outcome.clone().expect("outcome was set"))
        }
        Err(error) => {
            let error = error.to_string();
            state.error = Some(error.clone());
            Err(error)
        }
    }
}

fn progress_event(event: crate::progress::AgentProgress) -> Option<(String, String)> {
    use crate::progress::AgentProgress;
    match event {
        AgentProgress::TurnStarted => Some(("turn_started".into(), "Agent turn started".into())),
        AgentProgress::IterationStarted { iteration, .. } => {
            Some(("iteration_started".into(), format!("Iteration {iteration}")))
        }
        AgentProgress::ToolCallStarted { tool_name, .. } => {
            Some(("tool_call_started".into(), format!("Calling {tool_name}")))
        }
        AgentProgress::ToolCallCompleted {
            tool_name, success, ..
        } => Some((
            "tool_call_completed".into(),
            format!(
                "{tool_name} {}",
                if success { "completed" } else { "failed" }
            ),
        )),
        AgentProgress::TextDelta { delta, .. } if !delta.is_empty() => {
            Some(("text_delta".into(), delta))
        }
        AgentProgress::TurnCompleted { stop, iterations } => Some((
            "turn_completed".into(),
            stop.map_or_else(
                || format!("Turn completed after {iterations} iterations"),
                |stop| format!("Turn stopped: {}", stop.kind.as_str()),
            ),
        )),
        _ => None,
    }
}

fn epoch_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

fn bounded_event_text(text: String) -> String {
    const MAX_CHARS: usize = 4_096;
    if text.chars().count() <= MAX_CHARS {
        return text;
    }
    let mut bounded: String = text.chars().take(MAX_CHARS).collect();
    bounded.push_str("…(truncated)");
    bounded
}
