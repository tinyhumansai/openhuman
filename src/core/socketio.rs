use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use socketioxide::extract::{Data, SocketRef};
use socketioxide::SocketIo;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WebChannelEvent {
    pub event: String,
    pub client_id: String,
    pub thread_id: String,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_response: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct SocketRpcRequest {
    id: serde_json::Value,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct ChatStartPayload {
    thread_id: String,
    message: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    model_override: Option<String>,
    #[serde(default)]
    temperature: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct ChatCancelPayload {
    thread_id: String,
}

pub fn attach_socketio() -> (socketioxide::layer::SocketIoLayer, SocketIo) {
    let (layer, io) = SocketIo::new_layer();

    log::info!(
        "[socketio] engine ready (namespace /, path {})",
        io.config().engine_config.req_path
    );

    io.ns("/", |socket: SocketRef| {
        let client_id = socket.id.to_string();
        log::info!("[socketio] client connected id={client_id}");
        let _ = socket.join(client_id.clone());
        let ready_payload = json!({ "sid": client_id });
        log::debug!("[socketio] emit event=ready to_client={}", socket.id);
        let _ = socket.emit("ready", &ready_payload);

        socket.on("rpc:request", |socket: SocketRef, Data(payload): Data<SocketRpcRequest>| async move {
            let client_id = socket.id.to_string();
            log::info!(
                "[socketio] rpc:request method={} id={} client={}",
                payload.method,
                payload.id,
                client_id
            );
            log::debug!(
                "[socketio] rpc:request params_type={} params_bytes={}",
                json_type_name(&payload.params),
                payload.params.to_string().len()
            );

            let response = match crate::core::jsonrpc::invoke_method(
                crate::core::jsonrpc::default_state(),
                payload.method.as_str(),
                payload.params,
            )
            .await
            {
                Ok(result) => {
                    log::debug!(
                        "[socketio] send event=rpc:response client_id={} id={} result_type={} result_bytes={}",
                        client_id,
                        payload.id,
                        json_type_name(&result),
                        result.to_string().len()
                    );
                    ("rpc:response", json!({ "id": payload.id, "result": result }))
                }
                Err(message) => {
                    log::debug!(
                        "[socketio] send event=rpc:error client_id={} id={} message={}",
                        client_id,
                        payload.id,
                        message
                    );
                    (
                        "rpc:error",
                        json!({
                            "id": payload.id,
                            "error": { "code": -32000, "message": message }
                        }),
                    )
                }
            };

            let _ = socket.emit(response.0, &response.1);
        });

        socket.on("chat:start", |socket: SocketRef, Data(payload): Data<ChatStartPayload>| async move {
            let client_id = socket.id.to_string();
            let thread_id = payload.thread_id.clone();
            let model_override = payload.model_override.or(payload.model);
            log::debug!(
                "[socketio] recv event=chat:start client_id={} thread_id={} message_bytes={} model_override={:?} temperature={:?}",
                client_id,
                thread_id,
                payload.message.len(),
                model_override,
                payload.temperature
            );

            match crate::openhuman::channels::providers::web::start_chat(
                &client_id,
                &payload.thread_id,
                &payload.message,
                model_override,
                payload.temperature,
            )
            .await
            {
                Ok(request_id) => {
                    let accepted_payload = json!({
                        "event": "chat_accepted",
                        "client_id": client_id,
                        "thread_id": thread_id,
                        "request_id": request_id,
                    });
                    log::debug!("[socketio] send event=chat_accepted client_id={} thread_id={}", socket.id, payload.thread_id);
                    emit_with_aliases(&socket, "chat_accepted", &accepted_payload);
                }
                Err(error) => {
                    let error_payload = json!({
                    "event": "chat_error",
                    "client_id": client_id,
                    "thread_id": thread_id,
                    "request_id": "",
                    "message": error,
                    "error_type": "inference",
                });
                    log::debug!("[socketio] send event=chat_error client_id={} thread_id={} message={}", socket.id, payload.thread_id, error);
                    emit_with_aliases(&socket, "chat_error", &error_payload);
                }
            }
        });

        socket.on("chat:cancel", |socket: SocketRef, Data(payload): Data<ChatCancelPayload>| async move {
            let client_id = socket.id.to_string();
            log::debug!(
                "[socketio] recv event=chat:cancel client_id={} thread_id={}",
                client_id,
                payload.thread_id
            );
            let _ =
                crate::openhuman::channels::providers::web::cancel_chat(&client_id, &payload.thread_id)
                    .await;
        });
    });

    (layer, io)
}

pub fn spawn_web_channel_bridge(io: SocketIo) {
    tokio::spawn(async move {
        let mut rx = crate::openhuman::channels::providers::web::subscribe_web_channel_events();
        loop {
            let event = match rx.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!(
                        "[socketio] dropped {} web_channel events due to lag",
                        skipped
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };

            emit_web_channel_event(&io, event);
        }
        log::debug!("[socketio] web_channel bridge stopped");
    });
}

fn emit_web_channel_event(io: &SocketIo, event: WebChannelEvent) {
    let room = event.client_id.clone();
    let name = event.event.clone();
    if let Ok(payload) = serde_json::to_value(event) {
        log::debug!(
            "[socketio] send event={} room={} thread_id={} request_id={}",
            name,
            room,
            payload
                .get("thread_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default(),
            payload
                .get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
        );
        emit_room_with_aliases(io, &room, &name, &payload);
    }
}

fn event_alias(name: &str) -> Option<String> {
    if name.contains('_') {
        return Some(name.replace('_', ":"));
    }
    if name.contains(':') {
        return Some(name.replace(':', "_"));
    }
    None
}

fn emit_with_aliases(socket: &SocketRef, name: &str, payload: &serde_json::Value) {
    let _ = socket.emit(name, payload);
    if let Some(alias) = event_alias(name) {
        let _ = socket.emit(alias, payload);
    }
}

fn emit_room_with_aliases(io: &SocketIo, room: &str, name: &str, payload: &serde_json::Value) {
    let _ = io.to(room.to_string()).emit(name, payload);
    if let Some(alias) = event_alias(name) {
        let _ = io.to(room.to_string()).emit(alias, payload);
    }
}

fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::event_alias;

    #[test]
    fn event_alias_translates_between_delimiters() {
        assert_eq!(event_alias("chat_done").as_deref(), Some("chat:done"));
        assert_eq!(event_alias("chat:error").as_deref(), Some("chat_error"));
        assert_eq!(event_alias("ready"), None);
    }
}
