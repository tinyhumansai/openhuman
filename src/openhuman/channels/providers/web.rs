use once_cell::sync::Lazy;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use tokio::sync::{broadcast, Mutex};
use uuid::Uuid;

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::socketio::WebChannelEvent;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::agent::Agent;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::presentation;

static EVENT_BUS: Lazy<broadcast::Sender<WebChannelEvent>> = Lazy::new(|| {
    let (tx, _rx) = broadcast::channel(512);
    tx
});

pub fn subscribe_web_channel_events() -> broadcast::Receiver<WebChannelEvent> {
    EVENT_BUS.subscribe()
}

pub fn publish_web_channel_event(event: WebChannelEvent) {
    let _ = EVENT_BUS.send(event);
}

struct SessionEntry {
    agent: Agent,
    model_override: Option<String>,
    temperature: Option<f64>,
    /// Which agent definition was used to build `agent`. Recorded so
    /// that the cache hit predicate in `run_chat_task` can detect
    /// when the routing decision (welcome vs orchestrator) flips
    /// between turns and rebuild instead of reusing a stale agent.
    /// Without this field the cache hit short-circuited the routing
    /// fix from Commit 8 — the very first turn picked welcome,
    /// welcome called `complete_onboarding(complete)`, the flag
    /// flipped, but the next turn read the cached welcome agent
    /// instead of invoking `build_session_agent` to re-resolve the
    /// target.
    target_agent_id: String,
}

/// Decide which agent definition this turn should run with.
///
/// Mirrors the routing decision inside `build_session_agent` so
/// `run_chat_task` can compute it once up front and use it both as
/// the cache hit predicate AND (transitively) as the target id the
/// builder picks. Reads `chat_onboarding_completed` from a fresh
/// disk-loaded `Config` (no in-process cache) so the value reflects
/// the current persisted state — meaning the moment the welcome
/// agent calls `complete_onboarding(complete)` and the flag flips
/// to `true`, the very next chat turn observes the new value here
/// and the cache miss + rebuild routes to orchestrator.
fn pick_target_agent_id(config: &Config) -> &'static str {
    if config.chat_onboarding_completed {
        "orchestrator"
    } else {
        "welcome"
    }
}

#[derive(Debug)]
struct InFlightEntry {
    request_id: String,
    handle: tokio::task::JoinHandle<()>,
}

static THREAD_SESSIONS: Lazy<Mutex<HashMap<String, SessionEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static IN_FLIGHT: Lazy<Mutex<HashMap<String, InFlightEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn key_for(client_id: &str, thread_id: &str) -> String {
    format!("{client_id}::{thread_id}")
}

fn event_session_id_for(client_id: &str, thread_id: &str) -> String {
    json!({
        "client_id": client_id,
        "thread_id": thread_id,
    })
    .to_string()
}

pub async fn start_chat(
    client_id: &str,
    thread_id: &str,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
) -> Result<String, String> {
    let client_id = client_id.trim().to_string();
    let thread_id = thread_id.trim().to_string();
    let message = message.trim().to_string();

    if client_id.is_empty() {
        return Err("client_id is required".to_string());
    }
    if thread_id.is_empty() {
        return Err("thread_id is required".to_string());
    }
    if message.is_empty() {
        return Err("message is required".to_string());
    }

    let request_id = Uuid::new_v4().to_string();
    let map_key = key_for(&client_id, &thread_id);

    {
        let mut in_flight = IN_FLIGHT.lock().await;
        if let Some(existing) = in_flight.remove(&map_key) {
            existing.handle.abort();
            publish_web_channel_event(WebChannelEvent {
                event: "chat_error".to_string(),
                client_id: client_id.clone(),
                thread_id: thread_id.clone(),
                request_id: existing.request_id,
                full_response: None,
                message: Some("Cancelled by newer request".to_string()),
                error_type: Some("cancelled".to_string()),
                tool_name: None,
                skill_id: None,
                args: None,
                output: None,
                success: None,
                round: None,
                reaction_emoji: None,
                segment_index: None,
                segment_total: None,
                delta: None,
                delta_kind: None,
                tool_call_id: None,
            });
        }
    }

    let client_id_task = client_id.clone();
    let thread_id_task = thread_id.clone();
    let request_id_task = request_id.clone();
    let map_key_task = map_key.clone();

    let user_message = message.clone();
    let handle = tokio::spawn(async move {
        let result = run_chat_task(
            &client_id_task,
            &thread_id_task,
            &request_id_task,
            &user_message,
            model_override,
            temperature,
        )
        .await;

        match result {
            Ok(full_response) => {
                // ── Presentation layer (local model, fire-and-forget) ─────
                // Segment the response into human-readable bubbles and
                // decide whether to react — both run via local Ollama if
                // available, zero cloud cost.
                presentation::deliver_response(
                    &client_id_task,
                    &thread_id_task,
                    &request_id_task,
                    &full_response,
                    &user_message,
                )
                .await;
            }
            Err(err) => {
                publish_web_channel_event(WebChannelEvent {
                    event: "chat_error".to_string(),
                    client_id: client_id_task.clone(),
                    thread_id: thread_id_task.clone(),
                    request_id: request_id_task.clone(),
                    full_response: None,
                    message: Some(err),
                    error_type: Some("inference".to_string()),
                    tool_name: None,
                    skill_id: None,
                    args: None,
                    output: None,
                    success: None,
                    round: None,
                    reaction_emoji: None,
                    segment_index: None,
                    segment_total: None,
                    delta: None,
                    delta_kind: None,
                    tool_call_id: None,
                });
            }
        }

        let mut in_flight = IN_FLIGHT.lock().await;
        if let Some(current) = in_flight.get(&map_key_task) {
            if current.request_id == request_id_task {
                in_flight.remove(&map_key_task);
            }
        }
    });

    {
        let mut in_flight = IN_FLIGHT.lock().await;
        in_flight.insert(
            map_key,
            InFlightEntry {
                request_id: request_id.clone(),
                handle,
            },
        );
    }

    Ok(request_id)
}

pub async fn cancel_chat(client_id: &str, thread_id: &str) -> Result<Option<String>, String> {
    let client_id = client_id.trim();
    let thread_id = thread_id.trim();

    if client_id.is_empty() {
        return Err("client_id is required".to_string());
    }
    if thread_id.is_empty() {
        return Err("thread_id is required".to_string());
    }

    let map_key = key_for(client_id, thread_id);
    let mut removed_request_id: Option<String> = None;

    {
        let mut in_flight = IN_FLIGHT.lock().await;
        if let Some(existing) = in_flight.remove(&map_key) {
            removed_request_id = Some(existing.request_id.clone());
            existing.handle.abort();
        }
    }

    if let Some(request_id) = removed_request_id.clone() {
        publish_web_channel_event(WebChannelEvent {
            event: "chat_error".to_string(),
            client_id: client_id.to_string(),
            thread_id: thread_id.to_string(),
            request_id,
            full_response: None,
            message: Some("Cancelled".to_string()),
            error_type: Some("cancelled".to_string()),
            tool_name: None,
            skill_id: None,
            args: None,
            output: None,
            success: None,
            round: None,
            reaction_emoji: None,
            segment_index: None,
            segment_total: None,
            delta: None,
            delta_kind: None,
            tool_call_id: None,
        });
    }

    Ok(removed_request_id)
}

async fn run_chat_task(
    client_id: &str,
    thread_id: &str,
    request_id: &str,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
) -> Result<String, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let map_key = key_for(client_id, thread_id);
    let model_override = normalize_model_override(model_override);

    // Compute the routing decision up front so the cache lookup can
    // detect when it has changed. Without this, a turn that flips
    // `chat_onboarding_completed` (welcome agent calling
    // `complete_onboarding(complete)`) would still serve the next
    // turn from the cached welcome agent — the cache hit predicate
    // didn't know about the routing decision before Commit 13.
    let target_agent_id = pick_target_agent_id(&config).to_string();

    let prior = {
        let mut sessions = THREAD_SESSIONS.lock().await;
        sessions.remove(&map_key)
    };

    let mut agent = match prior {
        Some(entry)
            if entry.model_override == model_override
                && entry.temperature == temperature
                && entry.target_agent_id == target_agent_id =>
        {
            log::info!(
                "[web-channel] reusing cached session agent id={} for client={} thread={}",
                target_agent_id,
                client_id,
                thread_id
            );
            entry.agent
        }
        Some(prior_entry) => {
            log::info!(
                "[web-channel] cache miss — rebuilding session agent (was id={}, now id={}) for client={} thread={}",
                prior_entry.target_agent_id,
                target_agent_id,
                client_id,
                thread_id
            );
            build_session_agent(
                &config,
                client_id,
                thread_id,
                model_override.clone(),
                temperature,
            )?
        }
        None => build_session_agent(
            &config,
            client_id,
            thread_id,
            model_override.clone(),
            temperature,
        )?,
    };

    // Wire up a real-time progress channel so tool calls, iterations,
    // and sub-agent events are emitted to the web channel as they happen
    // (instead of retroactively after the loop finishes).
    let (progress_tx, progress_rx) = tokio::sync::mpsc::channel(64);
    agent.set_on_progress(Some(progress_tx));
    spawn_progress_bridge(
        progress_rx,
        client_id.to_string(),
        thread_id.to_string(),
        request_id.to_string(),
    );

    let result = agent.run_single(message).await.map_err(|e| e.to_string());

    // Clear the sender so it doesn't hold the channel open across sessions.
    agent.set_on_progress(None);

    {
        let mut sessions = THREAD_SESSIONS.lock().await;
        sessions.insert(
            map_key,
            SessionEntry {
                agent,
                model_override,
                temperature,
                target_agent_id,
            },
        );
    }

    result
}

/// Spawn a background task that reads [`AgentProgress`] events from the
/// agent turn loop and translates them into [`WebChannelEvent`]s tagged
/// with the correct client/thread/request IDs. The task runs until the
/// sender is dropped (i.e. when the agent turn finishes).
fn spawn_progress_bridge(
    mut rx: tokio::sync::mpsc::Receiver<crate::openhuman::agent::progress::AgentProgress>,
    client_id: String,
    thread_id: String,
    request_id: String,
) {
    use crate::openhuman::agent::progress::AgentProgress;

    tokio::spawn(async move {
        let mut round: u32 = 0;
        while let Some(event) = rx.recv().await {
            match event {
                AgentProgress::TurnStarted => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "inference_start".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        full_response: None,
                        message: None,
                        error_type: None,
                        tool_name: None,
                        skill_id: None,
                        args: None,
                        output: None,
                        success: None,
                        round: None,
                        reaction_emoji: None,
                        segment_index: None,
                        segment_total: None,
                        delta: None,
                        delta_kind: None,
                        tool_call_id: None,
                    });
                }
                AgentProgress::IterationStarted {
                    iteration,
                    max_iterations,
                } => {
                    round = iteration;
                    publish_web_channel_event(WebChannelEvent {
                        event: "iteration_start".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        full_response: None,
                        message: Some(format!("Iteration {iteration}/{max_iterations}")),
                        error_type: None,
                        tool_name: None,
                        skill_id: None,
                        args: None,
                        output: None,
                        success: None,
                        round: Some(iteration),
                        reaction_emoji: None,
                        segment_index: None,
                        segment_total: None,
                        delta: None,
                        delta_kind: None,
                        tool_call_id: None,
                    });
                }
                AgentProgress::ToolCallStarted {
                    call_id,
                    tool_name,
                    arguments,
                    iteration,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "tool_call".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: Some(tool_name),
                        skill_id: Some("web_channel".to_string()),
                        args: Some(arguments),
                        round: Some(iteration),
                        tool_call_id: Some(call_id),
                        ..Default::default()
                    });
                }
                AgentProgress::ToolCallCompleted {
                    call_id,
                    tool_name,
                    success,
                    output_chars,
                    elapsed_ms,
                    iteration,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "tool_result".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: Some(tool_name),
                        skill_id: Some("web_channel".to_string()),
                        output: Some(
                            json!({"output_chars": output_chars, "elapsed_ms": elapsed_ms})
                                .to_string(),
                        ),
                        success: Some(success),
                        round: Some(iteration),
                        tool_call_id: Some(call_id),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentSpawned { agent_id, task_id } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_spawned".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        full_response: None,
                        message: Some(format!("Sub-agent '{agent_id}' spawned")),
                        error_type: None,
                        tool_name: Some(agent_id),
                        skill_id: Some(task_id),
                        args: None,
                        output: None,
                        success: None,
                        round: Some(round),
                        reaction_emoji: None,
                        segment_index: None,
                        segment_total: None,
                        delta: None,
                        delta_kind: None,
                        tool_call_id: None,
                    });
                }
                AgentProgress::SubagentCompleted {
                    agent_id,
                    task_id,
                    elapsed_ms,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_completed".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        full_response: None,
                        message: Some(format!(
                            "Sub-agent '{agent_id}' completed in {elapsed_ms}ms"
                        )),
                        error_type: None,
                        tool_name: Some(agent_id),
                        skill_id: Some(task_id),
                        args: None,
                        output: None,
                        success: Some(true),
                        round: Some(round),
                        reaction_emoji: None,
                        segment_index: None,
                        segment_total: None,
                        delta: None,
                        delta_kind: None,
                        tool_call_id: None,
                    });
                }
                AgentProgress::SubagentFailed {
                    agent_id,
                    task_id,
                    error,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_failed".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        full_response: None,
                        message: Some(error),
                        error_type: None,
                        tool_name: Some(agent_id),
                        skill_id: Some(task_id),
                        args: None,
                        output: None,
                        success: Some(false),
                        round: Some(round),
                        reaction_emoji: None,
                        segment_index: None,
                        segment_total: None,
                        delta: None,
                        delta_kind: None,
                        tool_call_id: None,
                    });
                }
                AgentProgress::TextDelta { delta, iteration } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "text_delta".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        round: Some(iteration),
                        delta: Some(delta),
                        delta_kind: Some("text".to_string()),
                        ..Default::default()
                    });
                }
                AgentProgress::ThinkingDelta { delta, iteration } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "thinking_delta".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        round: Some(iteration),
                        delta: Some(delta),
                        delta_kind: Some("thinking".to_string()),
                        ..Default::default()
                    });
                }
                AgentProgress::ToolCallArgsDelta {
                    call_id,
                    tool_name,
                    delta,
                    iteration,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "tool_args_delta".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: if tool_name.is_empty() {
                            None
                        } else {
                            Some(tool_name)
                        },
                        skill_id: Some("web_channel".to_string()),
                        round: Some(iteration),
                        delta: Some(delta),
                        delta_kind: Some("tool_args".to_string()),
                        tool_call_id: Some(call_id),
                        ..Default::default()
                    });
                }
                AgentProgress::TurnCompleted { iterations } => {
                    log::debug!(
                        "[web_channel] turn completed after {iterations} iteration(s) \
                         client_id={client_id} thread_id={thread_id} request_id={request_id}"
                    );
                }
            }
        }
    });
}

fn normalize_model_override(model_override: Option<String>) -> Option<String> {
    model_override
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
}

fn build_session_agent(
    config: &Config,
    client_id: &str,
    thread_id: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
) -> Result<Agent, String> {
    let mut effective = config.clone();
    if let Some(model) = model_override {
        effective.default_model = Some(model);
    }
    if let Some(temp) = temperature {
        effective.default_temperature = temp;
    }

    // Route to welcome vs orchestrator based on the per-user
    // **chat-onboarding** flag. #525 fix: pre-onboarding users see the
    // welcome agent's persona with its 2-tool TOML scope
    // (complete_onboarding + memory_recall) instead of the
    // orchestrator's default delegation surface. Post-onboarding they
    // transition automatically on the next chat turn because
    // `Config::load_or_init` reads fresh from disk every call.
    //
    // We deliberately read `chat_onboarding_completed`, NOT
    // `onboarding_completed`. The latter is the React UI wizard's
    // gate (`OnboardingOverlay.tsx`) which flips to `true` the moment
    // the user dismisses the wizard — which happens BEFORE they ever
    // type in the chat pane. If we routed on that flag the welcome
    // agent could never run from the Tauri desktop app. The chat
    // flag is set only by the welcome agent itself via
    // `complete_onboarding(action="complete")`, so it stays `false`
    // for the user's actual first chat message regardless of what
    // the React layer did, then flips on the welcome turn so the
    // very next message routes to orchestrator.
    //
    // The config reached here has already been loaded by
    // `run_chat_task` via `config_rpc::load_config_with_timeout`, so
    // both flags reflect the current persisted state — no cache to
    // invalidate.
    let target_agent_id = if effective.chat_onboarding_completed {
        "orchestrator"
    } else {
        "welcome"
    };

    log::info!(
        "[web-channel] routing chat turn to '{}' (chat_onboarding_completed={}, ui_onboarding_completed={}, client_id={}, thread_id={})",
        target_agent_id,
        effective.chat_onboarding_completed,
        effective.onboarding_completed,
        client_id,
        thread_id
    );

    Agent::from_config_for_agent(&effective, target_agent_id)
        .map(|mut agent| {
            agent.set_event_context(event_session_id_for(client_id, thread_id), "web_channel");
            agent
        })
        .map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize)]
struct WebChatParams {
    client_id: String,
    thread_id: String,
    message: String,
    model_override: Option<String>,
    temperature: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct WebCancelParams {
    client_id: String,
    thread_id: String,
}

pub async fn channel_web_chat(
    client_id: &str,
    thread_id: &str,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
) -> Result<RpcOutcome<Value>, String> {
    let request_id = start_chat(client_id, thread_id, message, model_override, temperature).await?;

    Ok(RpcOutcome::single_log(
        json!({
            "accepted": true,
            "client_id": client_id.trim(),
            "thread_id": thread_id.trim(),
            "request_id": request_id,
        }),
        "web channel request accepted",
    ))
}

pub async fn channel_web_cancel(
    client_id: &str,
    thread_id: &str,
) -> Result<RpcOutcome<Value>, String> {
    let cancelled_request_id = cancel_chat(client_id, thread_id).await?;

    Ok(RpcOutcome::single_log(
        json!({
            "cancelled": cancelled_request_id.is_some(),
            "client_id": client_id.trim(),
            "thread_id": thread_id.trim(),
            "request_id": cancelled_request_id,
        }),
        "web channel cancellation processed",
    ))
}

pub fn all_web_channel_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("chat"), schemas("cancel")]
}

pub fn all_web_channel_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("chat"),
            handler: handle_chat,
        },
        RegisteredController {
            schema: schemas("cancel"),
            handler: handle_cancel,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "chat" => ControllerSchema {
            namespace: "channel",
            function: "web_chat",
            description: "Send a web channel message through the agent loop.",
            inputs: vec![
                required_string("client_id", "Client stream identifier."),
                required_string("thread_id", "Thread identifier."),
                required_string("message", "User message."),
                optional_string("model_override", "Optional model override."),
                optional_f64("temperature", "Optional temperature override."),
            ],
            outputs: vec![json_output("ack", "Acceptance payload.")],
        },
        "cancel" => ControllerSchema {
            namespace: "channel",
            function: "web_cancel",
            description: "Cancel in-flight web channel request for a thread.",
            inputs: vec![
                required_string("client_id", "Client stream identifier."),
                required_string("thread_id", "Thread identifier."),
            ],
            outputs: vec![json_output("ack", "Cancellation payload.")],
        },
        _ => ControllerSchema {
            namespace: "channel",
            function: "unknown",
            description: "Unknown web channel controller function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn handle_chat(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<WebChatParams>(params)?;
        to_json(
            channel_web_chat(
                &p.client_id,
                &p.thread_id,
                &p.message,
                p.model_override,
                p.temperature,
            )
            .await?,
        )
    })
}

fn handle_cancel(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<WebCancelParams>(params)?;
        to_json(channel_web_cancel(&p.client_id, &p.thread_id).await?)
    })
}

fn deserialize_params<T: serde::de::DeserializeOwned>(
    params: Map<String, Value>,
) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn optional_f64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
        comment,
        required: false,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::{cancel_chat, start_chat};

    #[tokio::test]
    async fn start_chat_validates_required_fields() {
        let err = start_chat("", "thread", "hello", None, None)
            .await
            .expect_err("client id should be required");
        assert!(err.contains("client_id is required"));

        let err = start_chat("client", "", "hello", None, None)
            .await
            .expect_err("thread id should be required");
        assert!(err.contains("thread_id is required"));

        let err = start_chat("client", "thread", "   ", None, None)
            .await
            .expect_err("message should be required");
        assert!(err.contains("message is required"));
    }

    #[tokio::test]
    async fn cancel_chat_validates_required_fields() {
        let err = cancel_chat("", "thread")
            .await
            .expect_err("client id should be required");
        assert!(err.contains("client_id is required"));

        let err = cancel_chat("client", "")
            .await
            .expect_err("thread id should be required");
        assert!(err.contains("thread_id is required"));
    }
}
