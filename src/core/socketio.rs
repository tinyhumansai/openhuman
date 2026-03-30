use serde::Deserialize;
use serde_json::json;
use socketioxide::extract::{Data, SocketRef};
use socketioxide::SocketIo;

use crate::openhuman::web_channel::events::WebChannelEvent;

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

    log::debug!(
        "[socketio] configured with req_path={}",
        io.config().engine_config.req_path
    );

    io.ns("/", |socket: SocketRef| {
        let client_id = socket.id.to_string();
        let _ = socket.join(client_id.clone());
        let ready_payload = json!({ "sid": client_id });
        let _ = socket.emit("ready", &ready_payload);

        socket.on("rpc:request", |socket: SocketRef, Data(payload): Data<SocketRpcRequest>| async move {
            let response = match crate::core::jsonrpc::invoke_method(
                crate::core::jsonrpc::default_state(),
                payload.method.as_str(),
                payload.params,
            )
            .await
            {
                Ok(result) => ("rpc:response", json!({ "id": payload.id, "result": result })),
                Err(message) => (
                    "rpc:error",
                    json!({
                        "id": payload.id,
                        "error": { "code": -32000, "message": message }
                    }),
                ),
            };

            let _ = socket.emit(response.0, &response.1);
        });

        socket.on("chat:start", |socket: SocketRef, Data(payload): Data<ChatStartPayload>| async move {
            let client_id = socket.id.to_string();
            let thread_id = payload.thread_id.clone();
            let model_override = payload.model_override.or(payload.model);

            if let Err(error) = crate::openhuman::web_channel::ops::channel_web_chat(
                &client_id,
                &payload.thread_id,
                &payload.message,
                model_override,
                payload.temperature,
            )
            .await
            {
                let error_payload = json!({
                    "event": "chat_error",
                    "client_id": client_id,
                    "thread_id": thread_id,
                    "request_id": "",
                    "message": error,
                    "error_type": "inference",
                });
                let _ = socket.emit("chat_error", &error_payload);
            }
        });

        socket.on("chat:cancel", |socket: SocketRef, Data(payload): Data<ChatCancelPayload>| async move {
            let client_id = socket.id.to_string();
            let _ = crate::openhuman::web_channel::ops::channel_web_cancel(
                &client_id,
                &payload.thread_id,
            )
            .await;
        });
    });

    (layer, io)
}

pub fn spawn_web_channel_bridge(io: SocketIo) {
    tokio::spawn(async move {
        let mut rx = crate::openhuman::web_channel::subscribe_web_channel_events();
        loop {
            let event = match rx.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!("[socketio] dropped {} web_channel events due to lag", skipped);
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };

            emit_web_channel_event(&io, event);
        }
    });
}

fn emit_web_channel_event(io: &SocketIo, event: WebChannelEvent) {
    let room = event.client_id.clone();
    let name = event.event.clone();
    if let Ok(payload) = serde_json::to_value(event) {
        let _ = io.to(room).emit(name, &payload);
    }
}
