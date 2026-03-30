use once_cell::sync::Lazy;
use std::collections::HashMap;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::openhuman::agent::Agent;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
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
        });
    }

    Ok(removed_request_id)
}

async fn run_chat_task(
    client_id: &str,
    thread_id: &str,
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

    let result = agent.run_single(message).await.map_err(|e| e.to_string());

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
