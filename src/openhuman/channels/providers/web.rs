use once_cell::sync::Lazy;
use serde_json::{Map, Value};
use std::collections::HashMap;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::openhuman::agent::Agent;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::openhuman::providers::ConversationMessage;
use crate::openhuman::web_channel::events::{publish, WebChannelEvent};

struct SessionEntry {
    agent: Agent,
    model_override: Option<String>,
    temperature: Option<f64>,
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
            publish(WebChannelEvent {
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
            });
        }
    }

    let client_id_task = client_id.clone();
    let thread_id_task = thread_id.clone();
    let request_id_task = request_id.clone();
    let map_key_task = map_key.clone();

    let handle = tokio::spawn(async move {
        let result = run_chat_task(
            &client_id_task,
            &thread_id_task,
            &request_id_task,
            &message,
            model_override,
            temperature,
        )
        .await;

        match result {
            Ok(full_response) => {
                publish(WebChannelEvent {
                    event: "chat_done".to_string(),
                    client_id: client_id_task.clone(),
                    thread_id: thread_id_task.clone(),
                    request_id: request_id_task.clone(),
                    full_response: Some(full_response),
                    message: None,
                    error_type: None,
                    tool_name: None,
                    skill_id: None,
                    args: None,
                    output: None,
                    success: None,
                    round: None,
                });
            }
            Err(err) => {
                publish(WebChannelEvent {
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
        publish(WebChannelEvent {
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

    let prior = {
        let mut sessions = THREAD_SESSIONS.lock().await;
        sessions.remove(&map_key)
    };

    let mut agent = match prior {
        Some(entry)
            if entry.model_override == model_override && entry.temperature == temperature =>
        {
            entry.agent
        }
        Some(_) | None => build_session_agent(&config, model_override.clone(), temperature)?,
    };

    let history_before = agent.history().len();
    let result = agent.run_single(message).await.map_err(|e| e.to_string());
    if result.is_ok() {
        publish_tool_events_from_history(
            client_id,
            thread_id,
            request_id,
            &agent.history()[history_before..],
        );
    }

    {
        let mut sessions = THREAD_SESSIONS.lock().await;
        sessions.insert(
            map_key,
            SessionEntry {
                agent,
                model_override,
                temperature,
            },
        );
    }

    result
}

fn publish_tool_events_from_history(
    client_id: &str,
    thread_id: &str,
    request_id: &str,
    messages: &[ConversationMessage],
) {
    let mut round: u32 = 0;
    let mut current_round_calls: Vec<(String, String)> = Vec::new();

    for message in messages {
        match message {
            ConversationMessage::AssistantToolCalls { tool_calls, .. } => {
                round += 1;
                current_round_calls.clear();
                for (idx, call) in tool_calls.iter().enumerate() {
                    let synthetic_id = if call.id.trim().is_empty() {
                        format!("idx:{idx}")
                    } else {
                        call.id.clone()
                    };
                    current_round_calls.push((synthetic_id, call.name.clone()));
                    publish(WebChannelEvent {
                        event: "tool_call".to_string(),
                        client_id: client_id.to_string(),
                        thread_id: thread_id.to_string(),
                        request_id: request_id.to_string(),
                        full_response: None,
                        message: None,
                        error_type: None,
                        tool_name: Some(call.name.clone()),
                        skill_id: Some("web_channel".to_string()),
                        args: Some(parse_tool_args(&call.arguments)),
                        output: None,
                        success: None,
                        round: Some(round),
                    });
                }
            }
            ConversationMessage::ToolResults(results) => {
                for (idx, result) in results.iter().enumerate() {
                    let fallback = format!("idx:{idx}");
                    let tool_name = current_round_calls
                        .iter()
                        .find(|(tool_call_id, _)| tool_call_id == &result.tool_call_id)
                        .or_else(|| current_round_calls.get(idx))
                        .map(|(_, name)| name.clone())
                        .unwrap_or_else(|| "unknown".to_string());

                    let success = !result.content.trim_start().starts_with("Error:");
                    publish(WebChannelEvent {
                        event: "tool_result".to_string(),
                        client_id: client_id.to_string(),
                        thread_id: thread_id.to_string(),
                        request_id: request_id.to_string(),
                        full_response: None,
                        message: None,
                        error_type: None,
                        tool_name: Some(tool_name),
                        skill_id: Some("web_channel".to_string()),
                        args: Some(Value::Object(Map::from_iter([(
                            "tool_call_id".to_string(),
                            Value::String(if result.tool_call_id.is_empty() {
                                fallback
                            } else {
                                result.tool_call_id.clone()
                            }),
                        )]))),
                        output: Some(result.content.clone()),
                        success: Some(success),
                        round: Some(round.max(1)),
                    });
                }
            }
            ConversationMessage::Chat(_) => {}
        }
    }
}

fn parse_tool_args(arguments: &str) -> Value {
    if arguments.trim().is_empty() {
        return Value::Object(Map::new());
    }
    match serde_json::from_str::<Value>(arguments) {
        Ok(value) => value,
        Err(_) => Value::Object(Map::from_iter([(
            "raw".to_string(),
            Value::String(arguments.to_string()),
        )])),
    }
}

fn normalize_model_override(model_override: Option<String>) -> Option<String> {
    model_override
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
}

fn build_session_agent(
    config: &Config,
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

    Agent::from_config(&effective).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{cancel_chat, parse_tool_args, start_chat};
    use serde_json::json;

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

    #[test]
    fn parse_tool_args_handles_json_and_raw_fallback() {
        assert_eq!(
            parse_tool_args(r#"{"command":"date"}"#),
            json!({"command":"date"})
        );
        assert_eq!(parse_tool_args(""), json!({}));
        assert_eq!(parse_tool_args("not-json"), json!({"raw":"not-json"}));
    }
}
