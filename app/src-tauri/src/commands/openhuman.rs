use crate::core_process::CoreProcessHandle;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Mutex;
use uuid::Uuid;

const DAEMON_HOST_CONFIG_FILE: &str = "daemon_host_config.json";

static WEB_CHAT_CLIENT_ID: Lazy<String> = Lazy::new(|| Uuid::new_v4().to_string());
static WEB_CHAT_STREAM_TASK: Lazy<Mutex<Option<tokio::task::JoinHandle<()>>>> =
    Lazy::new(|| Mutex::new(None));

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonHostConfig {
    pub show_tray: bool,
}

impl Default for DaemonHostConfig {
    fn default() -> Self {
        Self { show_tray: true }
    }
}

fn daemon_host_config_path(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".openhuman")
        })
        .join(DAEMON_HOST_CONFIG_FILE)
}

async fn load_daemon_host_config(app: &AppHandle) -> DaemonHostConfig {
    let path = daemon_host_config_path(app);
    let Ok(contents) = tokio::fs::read_to_string(path).await else {
        return DaemonHostConfig::default();
    };
    serde_json::from_str::<DaemonHostConfig>(&contents).unwrap_or_default()
}

async fn save_daemon_host_config(app: &AppHandle, config: &DaemonHostConfig) -> Result<(), String> {
    let path = daemon_host_config_path(app);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create daemon host config directory: {e}"))?;
    }
    let bytes = serde_json::to_vec_pretty(config)
        .map_err(|e| format!("failed to serialize daemon host config: {e}"))?;
    tokio::fs::write(path, bytes)
        .await
        .map_err(|e| format!("failed to write daemon host config: {e}"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceState {
    Running,
    Stopped,
    NotInstalled,
    Unknown(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub state: ServiceState,
    pub unit_path: Option<std::path::PathBuf>,
    pub label: String,
    pub details: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RpcCommandResponse<T> {
    result: T,
}

async fn ensure_core_running(app: &AppHandle) -> Result<(), String> {
    let core = app
        .try_state::<CoreProcessHandle>()
        .ok_or_else(|| "core process handle is not available".to_string())?;
    let handle: CoreProcessHandle = (*core).clone();
    handle.ensure_running().await
}

async fn call_service_method(app: &AppHandle, method: &str) -> Result<ServiceStatus, String> {
    ensure_core_running(app).await?;
    let response =
        crate::core_rpc::call::<RpcCommandResponse<ServiceStatus>>(method, serde_json::json!({}))
            .await?;
    Ok(response.result)
}

#[derive(Debug, Deserialize)]
struct WebChatSseEvent {
    event: String,
    client_id: String,
    thread_id: String,
    request_id: String,
    full_response: Option<String>,
    message: Option<String>,
    error_type: Option<String>,
    tool_name: Option<String>,
    skill_id: Option<String>,
    args: Option<Value>,
    output: Option<String>,
    success: Option<bool>,
    round: Option<u32>,
}

fn core_events_url() -> String {
    let rpc_url = crate::core_rpc::resolved_rpc_url();
    if rpc_url.ends_with("/rpc") {
        rpc_url.trim_end_matches("/rpc").to_string() + "/events"
    } else {
        rpc_url + "/events"
    }
}

fn parse_sse_block(block: &str) -> Option<WebChatSseEvent> {
    let mut event_name: Option<String> = None;
    let mut data_lines: Vec<&str> = Vec::new();
    for line in block.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event_name = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start());
        }
    }

    if data_lines.is_empty() {
        return None;
    }

    let data = data_lines.join("\n");
    let mut parsed: WebChatSseEvent = serde_json::from_str(&data).ok()?;
    if parsed.event.is_empty() {
        parsed.event = event_name.unwrap_or_default();
    }
    Some(parsed)
}

fn emit_web_chat_event(app: &AppHandle, event: WebChatSseEvent) {
    if event.client_id != *WEB_CHAT_CLIENT_ID {
        return;
    }

    match event.event.as_str() {
        "chat_done" => {
            let payload = serde_json::json!({
                "thread_id": event.thread_id,
                "full_response": event.full_response.unwrap_or_default(),
                "rounds_used": 0,
                "total_input_tokens": 0,
                "total_output_tokens": 0,
                "request_id": event.request_id,
            });
            let _ = app.emit("chat:done", payload);
        }
        "chat_error" => {
            let payload = serde_json::json!({
                "thread_id": event.thread_id,
                "message": event.message.unwrap_or_else(|| "Unknown chat error".to_string()),
                "error_type": event.error_type.unwrap_or_else(|| "inference".to_string()),
                "round": serde_json::Value::Null,
                "request_id": event.request_id,
            });
            let _ = app.emit("chat:error", payload);
        }
        "tool_call" => {
            let payload = serde_json::json!({
                "thread_id": event.thread_id,
                "tool_name": event.tool_name.unwrap_or_else(|| "unknown".to_string()),
                "skill_id": event.skill_id.unwrap_or_else(|| "web_channel".to_string()),
                "args": event.args.unwrap_or_else(|| serde_json::json!({})),
                "round": event.round.unwrap_or(0),
                "request_id": event.request_id,
            });
            let _ = app.emit("chat:tool_call", payload);
        }
        "tool_result" => {
            let payload = serde_json::json!({
                "thread_id": event.thread_id,
                "tool_name": event.tool_name.unwrap_or_else(|| "unknown".to_string()),
                "skill_id": event.skill_id.unwrap_or_else(|| "web_channel".to_string()),
                "output": event.output.unwrap_or_default(),
                "success": event.success.unwrap_or(true),
                "round": event.round.unwrap_or(0),
                "request_id": event.request_id,
            });
            let _ = app.emit("chat:tool_result", payload);
        }
        _ => {}
    }
}

async fn ensure_web_chat_stream(app: &AppHandle) -> Result<(), String> {
    let mut guard = WEB_CHAT_STREAM_TASK.lock().await;
    if guard.is_some() {
        return Ok(());
    }

    let app_handle = app.clone();
    let client_id = WEB_CHAT_CLIENT_ID.clone();
    let url = format!(
        "{}?client_id={}",
        core_events_url(),
        urlencoding::encode(&client_id)
    );

    let task = tokio::spawn(async move {
        let client = reqwest::Client::new();
        let mut backoff_ms: u64 = 500;

        loop {
            let response = match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => resp,
                Ok(resp) => {
                    log::warn!(
                        "[web-channel] SSE stream HTTP error status={} for {}",
                        resp.status(),
                        url
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                    backoff_ms = (backoff_ms * 2).min(10_000);
                    continue;
                }
                Err(err) => {
                    log::warn!("[web-channel] SSE connect error: {}", err);
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                    backoff_ms = (backoff_ms * 2).min(10_000);
                    continue;
                }
            };

            backoff_ms = 500;
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();

            use futures_util::StreamExt as _;
            while let Some(item) = stream.next().await {
                let chunk = match item {
                    Ok(chunk) => chunk,
                    Err(err) => {
                        log::warn!("[web-channel] SSE stream read error: {}", err);
                        break;
                    }
                };

                let text = match std::str::from_utf8(&chunk) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                buffer.push_str(text);

                while let Some(split_index) = buffer.find("\n\n") {
                    let block = buffer[..split_index].to_string();
                    buffer = buffer[split_index + 2..].to_string();
                    if let Some(event) = parse_sse_block(block.trim()) {
                        emit_web_chat_event(&app_handle, event);
                    }
                }
            }
        }
    });

    *guard = Some(task);
    Ok(())
}

#[tauri::command]
pub async fn chat_send(
    app: AppHandle,
    thread_id: String,
    message: String,
    model: String,
    auth_token: String,
    backend_url: String,
    messages: Vec<Value>,
    notion_context: Option<String>,
) -> Result<(), String> {
    ensure_web_chat_stream(&app).await?;

    let _ = (&auth_token, &backend_url, &messages, &notion_context);

    let params = serde_json::json!({
        "client_id": WEB_CHAT_CLIENT_ID.as_str(),
        "thread_id": thread_id,
        "message": message,
        "model_override": model,
    });

    let _: Value = crate::core_rpc::call("openhuman.channel_web_chat", params).await?;
    Ok(())
}

#[tauri::command]
pub async fn chat_cancel(thread_id: String) -> Result<bool, String> {
    let params = serde_json::json!({
        "client_id": WEB_CHAT_CLIENT_ID.as_str(),
        "thread_id": thread_id,
    });
    let response: Value = crate::core_rpc::call("openhuman.channel_web_cancel", params).await?;
    let cancelled = response
        .get("result")
        .and_then(|v| v.get("cancelled"))
        .or_else(|| response.get("cancelled"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(cancelled)
}

#[tauri::command]
pub async fn openhuman_get_daemon_host_config(app: AppHandle) -> Result<DaemonHostConfig, String> {
    Ok(load_daemon_host_config(&app).await)
}

#[tauri::command]
pub async fn openhuman_set_daemon_host_config(
    app: AppHandle,
    show_tray: bool,
) -> Result<DaemonHostConfig, String> {
    let mut cfg = load_daemon_host_config(&app).await;
    cfg.show_tray = show_tray;
    save_daemon_host_config(&app, &cfg).await?;
    Ok(cfg)
}

#[tauri::command]
pub async fn openhuman_service_install(app: AppHandle) -> Result<ServiceStatus, String> {
    call_service_method(&app, "openhuman.service_install").await
}

#[tauri::command]
pub async fn openhuman_service_start(app: AppHandle) -> Result<ServiceStatus, String> {
    call_service_method(&app, "openhuman.service_start").await
}

#[tauri::command]
pub async fn openhuman_service_stop(app: AppHandle) -> Result<ServiceStatus, String> {
    call_service_method(&app, "openhuman.service_stop").await
}

#[tauri::command]
pub async fn openhuman_service_status(app: AppHandle) -> Result<ServiceStatus, String> {
    call_service_method(&app, "openhuman.service_status").await
}

#[tauri::command]
pub async fn openhuman_service_uninstall(app: AppHandle) -> Result<ServiceStatus, String> {
    call_service_method(&app, "openhuman.service_uninstall").await
}
