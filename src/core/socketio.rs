use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use socketioxide::extract::{Data, SocketRef, TryData};
use socketioxide::SocketIo;

/// Marker stored in [`SocketRef::extensions`] once a connection has presented a
/// bearer token that matches the active per-process RPC token.
///
/// Event handlers consult this before forwarding attacker-controllable input
/// into the JSON-RPC dispatcher or the web-chat orchestrator: an unauthenticated
/// socket that never picked up the marker is allowed to receive broadcast-style
/// events (read-only) but cannot trigger executable work.
#[derive(Clone, Copy, Debug)]
struct AuthedConnection;

/// Connection-time payload the client passes via Socket.IO's `auth` field.
///
/// Browsers do not let `EventSource` / `WebSocket` clients attach custom
/// headers, so the handshake `auth` map is the only header-equivalent slot
/// available for our per-process bearer. The socket-IO Node/JS clients all
/// surface `io(url, { auth: { token: "<hex>" } })` for this.
#[derive(Debug, Default, Deserialize)]
struct HandshakeAuth {
    #[serde(default)]
    token: Option<String>,
}

/// Origins the local core trusts at the Socket.IO handshake.
///
/// The document origin of the CEF-served app shell is platform-dependent:
///
/// | Platform | Scheme | Host |
/// |----------|--------|------|
/// | macOS / iOS (native scheme) | `tauri` | `localhost` |
/// | Windows (CEF http custom protocol) | `http` | `tauri.localhost` |
/// | Linux / older Windows builds | `https` | `tauri.localhost` |
/// | Vite dev (`pnpm dev:app`, `pnpm dev`) | `http` | `localhost` / `127.0.0.1` / `[::1]` |
///
/// The handshake `Origin` header is stamped by the webview with whichever
/// of these shapes loaded the page — it is **not** the destination URL the
/// socket is connecting to. We match the parsed host against the allowlist
/// so all four shapes pass regardless of scheme, while `starts_with` decoys
/// like `http://localhost.attacker.example` are still rejected (parser
/// returns a different `host_str`).
///
/// A missing `Origin` header is treated as a native (non-browser) client
/// and accepted — only the cross-origin browser-page case is the targeted
/// bad actor here.
fn origin_is_allowed(origin: Option<&str>) -> bool {
    let Some(origin) = origin else {
        return true; // native clients (CLI, Tauri shell) — no Origin header
    };
    let origin = origin.trim();
    if origin.is_empty() || origin == "null" {
        return false;
    }
    // Parse the URL and compare the host EXACTLY against the loopback +
    // tauri.localhost allowlist. The earlier scheme-literal short-circuit
    // (`tauri://localhost` / `https://tauri.localhost`) missed
    // `http://tauri.localhost`, which is the document origin CEF stamps
    // on Windows — every flavour of the Tauri webview shell now goes
    // through the same host check.
    let Ok(parsed) = url::Url::parse(origin) else {
        return false;
    };
    // `url::Url::host_str` returns IPv6 hosts with surrounding brackets,
    // hostnames bare. Accept both shapes.
    matches!(
        parsed.host_str(),
        Some("localhost" | "127.0.0.1" | "::1" | "[::1]" | "tauri.localhost")
    )
}

/// True when `socket` finished the handshake with a valid bearer token.
fn socket_is_authed(socket: &SocketRef) -> bool {
    socket.extensions.get::<AuthedConnection>().is_some()
}

/// Best-effort disconnect. Called when we discover an unauthenticated socket
/// inside an event handler — the connect path already disconnects the bad
/// origins / wrong tokens, so this is purely a defense-in-depth path.
fn drop_unauthed(socket: &SocketRef, reason: &'static str) {
    log::warn!(
        "[socketio] dropping unauthenticated socket id={} reason={}",
        socket.id,
        reason
    );
    let _ = socket.clone().disconnect();
}

/// Standard event payload for the web channel transport.
///
/// This structure defines the data sent to Socket.IO clients for various
/// chat-related events, such as message delivery, tool execution, and errors.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WebChannelEvent {
    /// The event name (e.g., `chat_message`, `tool_call`).
    pub event: String,
    /// Unique identifier for the Socket.IO client.
    pub client_id: String,
    /// Identifier for the specific chat thread.
    pub thread_id: String,
    /// Unique identifier for the individual request/turn.
    pub request_id: String,
    /// The full text of the assistant's response (sent on completion).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_response: Option<String>,
    /// A partial message segment or an error description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Type of error, if the event represents a failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
    /// Name of the tool being called.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// ID of the skill owning the tool.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_id: Option<String>,
    /// Arguments passed to the tool.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<serde_json::Value>,
    /// The raw output from the tool execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Whether the tool execution or request was successful.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    /// The current iteration/round number in a tool-call loop.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round: Option<u32>,
    /// Emoji reaction the assistant wants to add to the user's message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reaction_emoji: Option<String>,
    /// 0-based index when a response is delivered as multiple segments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_index: Option<u32>,
    /// Total number of segments in a segmented delivery.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_total: Option<u32>,
    /// Fine-grained streaming payload for `text_delta`, `thinking_delta`,
    /// and `tool_args_delta` events. Concatenating `delta`s in order
    /// yields the full text/thinking/arguments string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    /// Discriminator for the `delta` payload: `"text"`, `"thinking"`,
    /// or `"tool_args"`. Only set on streaming delta events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta_kind: Option<String>,
    /// Provider-assigned tool call id that groups `tool_args_delta`
    /// chunks together and ties them to the eventual `tool_call` /
    /// `tool_result` events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Optional citations attached to `chat_done` payloads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citations: Option<serde_json::Value>,
    /// Sub-agent specific progress detail. Populated on
    /// `subagent_spawned`, `subagent_completed`, `subagent_iteration_start`,
    /// `subagent_tool_call`, and `subagent_tool_result` events so the UI
    /// can attribute child activity to the parent's live subagent row
    /// without overloading the flat top-level fields. `None` for any
    /// non-subagent event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent: Option<SubagentProgressDetail>,
    /// Per-thread task board snapshot carried by `task_board_updated`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_board: Option<serde_json::Value>,
}

/// Per-event subagent progress detail attached to `WebChannelEvent`.
///
/// Carries the fields the parent thread's UI needs to render a live
/// subagent block — child iteration counters, mode, child task/agent
/// ids when distinct from the flat `tool_name` (which already carries
/// the agent id on top-level subagent events but not on nested
/// `subagent_tool_*` events where `tool_name` is the *child's* tool),
/// and final-run statistics on `subagent_completed`.
///
/// Every field is optional and skipped from the JSON payload when
/// absent — this keeps the wire format compact for non-subagent events
/// (where the whole struct is `None`) and lets new fields land
/// non-breakingly behind older clients.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SubagentProgressDetail {
    /// Resolved spawn mode — `"typed"` or `"fork"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Whether the spawn requested a dedicated worker thread.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dedicated_thread: Option<bool>,
    /// Character length of the delegation prompt (on `subagent_spawned`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_chars: Option<u64>,
    /// Sub-agent's child iteration counter (on `subagent_iteration_start`,
    /// `subagent_tool_call`, `subagent_tool_result`). 1-based.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_iteration: Option<u32>,
    /// Sub-agent's configured iteration cap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_max_iterations: Option<u32>,
    /// Child agent id (on nested `subagent_tool_*` events where the flat
    /// `tool_name` is the child's tool, not the agent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Spawn task id (on nested `subagent_tool_*` events).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Elapsed wall-clock for the call/run in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    /// Total iterations the sub-agent used (on `subagent_completed`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u32>,
    /// Character length of the sub-agent's final assistant text
    /// (on `subagent_completed`) or the tool result
    /// (on `subagent_tool_result`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_chars: Option<u64>,
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
    #[serde(default)]
    profile_id: Option<String>,
    #[serde(default)]
    locale: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatCancelPayload {
    thread_id: String,
}

/// Attaches the Socket.IO layer to the Axum router and sets up event handlers.
///
/// It configures:
/// - Client connection and room joining.
/// - `rpc:request`: Invoking JSON-RPC methods over WebSocket.
/// - `chat:start`: Initiating a new chat turn.
/// - `chat:cancel`: Aborting an active chat turn.
pub fn attach_socketio() -> (socketioxide::layer::SocketIoLayer, SocketIo) {
    let (layer, io) = SocketIo::new_layer();

    log::info!(
        "[socketio] engine ready (namespace /, path {})",
        io.config().engine_config.req_path
    );

    io.ns(
        "/",
        |socket: SocketRef, TryData(handshake): TryData<HandshakeAuth>| {
            let client_id = socket.id.to_string();

            // Reject cross-origin browser pages before the handshake completes.
            // Native clients (Tauri shell, CLI) do not set an `Origin` header and
            // are accepted; only browser pages from origins outside the local
            // app surface are dropped here. See `origin_is_allowed`.
            let origin = socket
                .req_parts()
                .headers
                .get(axum::http::header::ORIGIN)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            if !origin_is_allowed(origin.as_deref()) {
                log::warn!(
                    "[socketio] rejecting connect: bad origin {:?} client={}",
                    origin,
                    client_id
                );
                let _ = socket.clone().disconnect();
                return;
            }

            // Verify the handshake bearer matches the per-process RPC token.
            // `TryData` lets us treat a missing/malformed `auth` payload as a
            // soft failure (no panic) and reject the connect cleanly.
            let supplied = handshake.ok().and_then(|h| h.token).unwrap_or_default();
            if !crate::core::auth::verify_bearer_token(&supplied) {
                log::warn!(
                    "[socketio] rejecting connect: missing or invalid bearer client={}",
                    client_id
                );
                let _ = socket.clone().disconnect();
                return;
            }
            socket.extensions.insert(AuthedConnection);

            log::info!("[socketio] client connected id={client_id} (authenticated)");
            // Join a room named after the client ID for targeted event delivery.
            join_room_logged(&socket, &client_id, &client_id);
            // Also auto-join the "system" room so every connected client
            // receives broadcast-style events that aren't tied to a
            // specific chat thread. Today this covers proactive messages
            // (welcome agent, morning briefing, cron-driven announcements)
            // which `channels::proactive::ProactiveMessageSubscriber`
            // emits with `client_id = "system"` — see `emit_web_channel_event`.
            // If this join fails the welcome message silently disappears,
            // so we log both success and failure for diagnosability.
            join_room_logged(&socket, "system", &client_id);
            let ready_payload = json!({ "sid": client_id });
            log::debug!("[socketio] emit event=ready to_client={}", socket.id);
            let _ = socket.emit("ready", &ready_payload);

            // Handler for JSON-RPC over WebSocket.
            socket.on(
                "rpc:request",
                |socket: SocketRef, Data(payload): Data<SocketRpcRequest>| async move {
                    if !socket_is_authed(&socket) {
                        drop_unauthed(&socket, "rpc:request from unauthenticated socket");
                        return;
                    }
                    let client_id = socket.id.to_string();
                    log::info!(
                        "[socketio] rpc:request method={} id={} client={}",
                        payload.method,
                        payload.id,
                        client_id
                    );

                    // Invoke the method through the same logic used by the HTTP RPC endpoint.
                    let response = match crate::core::jsonrpc::invoke_method(
                        crate::core::jsonrpc::default_state(),
                        payload.method.as_str(),
                        payload.params,
                    )
                    .await
                    {
                        Ok(result) => (
                            "rpc:response",
                            json!({ "id": payload.id, "result": result }),
                        ),
                        Err(message) => (
                            "rpc:error",
                            json!({
                                "id": payload.id,
                                "error": { "code": -32000, "message": message }
                            }),
                        ),
                    };

                    let _ = socket.emit(response.0, &response.1);
                },
            );

            // Handler for starting a chat turn.
            socket.on(
                "chat:start",
                |socket: SocketRef, Data(payload): Data<ChatStartPayload>| async move {
                    if !socket_is_authed(&socket) {
                        drop_unauthed(&socket, "chat:start from unauthenticated socket");
                        return;
                    }
                    let client_id = socket.id.to_string();
                    let thread_id = payload.thread_id.clone();
                    let model_override = payload.model_override.or(payload.model);
                    log::debug!(
                    "[socketio] recv event=chat:start client_id={} thread_id={} message_bytes={}",
                    client_id,
                    thread_id,
                    payload.message.len()
                );

                    // Trigger the web channel's chat logic.
                    match crate::openhuman::channels::providers::web::start_chat(
                        &client_id,
                        &payload.thread_id,
                        &payload.message,
                        model_override,
                        payload.temperature,
                        payload.profile_id,
                        payload.locale,
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
                            emit_with_aliases(&socket, "chat_error", &error_payload);
                        }
                    }
                },
            );

            // Handler for cancelling an active chat turn.
            socket.on(
                "chat:cancel",
                |socket: SocketRef, Data(payload): Data<ChatCancelPayload>| async move {
                    if !socket_is_authed(&socket) {
                        drop_unauthed(&socket, "chat:cancel from unauthenticated socket");
                        return;
                    }
                    let client_id = socket.id.to_string();
                    log::debug!(
                        "[socketio] recv event=chat:cancel client_id={} thread_id={}",
                        client_id,
                        payload.thread_id
                    );
                    let _ = crate::openhuman::channels::providers::web::cancel_chat(
                        &client_id,
                        &payload.thread_id,
                    )
                    .await;
                },
            );
        },
    );

    (layer, io)
}

/// Spawns background bridges to forward various system events to Socket.IO clients.
///
/// This function sets up five bridges:
/// 1. **Web Channel Bridge**: Forwards chat-related events (messages, tool calls) to specific clients.
/// 2. **Dictation Bridge**: Forwards hotkey events to all clients.
/// 3. **Overlay Bridge**: Forwards attention bubble events to all clients.
/// 4. **Core Notification Bridge**: Forwards core notification events to all clients.
/// 5. **Transcription Bridge**: Forwards real-time speech-to-text results to all clients.
pub fn spawn_web_channel_bridge(io: SocketIo) {
    // 1. Web channel events → per-client rooms.
    let io_web = io.clone();
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

            emit_web_channel_event(&io_web, event);
        }
        log::debug!("[socketio] web_channel bridge stopped");
    });

    let io_overlay = io.clone();
    let io_notify = io.clone();
    let io_transcription = io.clone();
    let io_auth = io.clone();
    let io_companion = io.clone();

    // 2. Dictation hotkey events → broadcast to all connected clients.
    tokio::spawn(async move {
        let mut rx = crate::openhuman::voice::dictation_listener::subscribe_dictation_events();
        loop {
            let event = match rx.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!("[socketio] dropped {} dictation events due to lag", skipped);
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };

            if let Ok(payload) = serde_json::to_value(&event) {
                log::debug!(
                    "[socketio] broadcast dictation:{} to all clients",
                    event.event_type
                );
                // Support both colon and underscore versions for compatibility with different frontends.
                let _ = io.emit("dictation:toggle", &payload);
                let _ = io.emit("dictation_toggle", &payload);
            }
        }
        log::debug!("[socketio] dictation bridge stopped");
    });

    // 3. Overlay attention events → broadcast to all clients.
    tokio::spawn(async move {
        let mut rx = crate::openhuman::overlay::subscribe_attention_events();
        loop {
            let event = match rx.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!(
                        "[socketio] dropped {} overlay attention events due to lag",
                        skipped
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };

            if let Ok(payload) = serde_json::to_value(&event) {
                log::debug!(
                    "[socketio] broadcast overlay:attention source={:?}",
                    event.source
                );
                let _ = io_overlay.emit("overlay:attention", &payload);
                let _ = io_overlay.emit("overlay_attention", &payload);
            }
        }
        log::debug!("[socketio] overlay attention bridge stopped");
    });

    // 4. Core notification events → broadcast to all connected clients so
    //    the in-app notification center picks them up regardless of which
    //    chat session is active. Pattern mirrors the overlay attention
    //    bridge above — fire-and-forget, no per-client routing.
    tokio::spawn(async move {
        let mut rx = crate::openhuman::notifications::subscribe_core_notifications();
        loop {
            let event = match rx.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!(
                        "[socketio] dropped {} core_notification events due to lag",
                        skipped
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };

            if let Ok(payload) = serde_json::to_value(&event) {
                log::debug!(
                    "[socketio] broadcast core_notification id={} category={:?}",
                    event.id,
                    event.category
                );
                let _ = io_notify.emit("core_notification", &payload);
                let _ = io_notify.emit("core:notification", &payload);
            }
        }
        log::debug!("[socketio] core_notification bridge stopped");
    });

    // 6. SessionExpired events → broadcast to all clients so the UI can
    //    proactively tear down user-scoped state and route to onboarding
    //    instead of waiting for the next poll to discover the JWT is gone.
    //    Subscribes to the global event bus and filters for
    //    `DomainEvent::SessionExpired`; ignores everything else.
    tokio::spawn(async move {
        // Poll until `event_bus::init_global` has run. Socket.IO bridges
        // spawn from `spawn_web_channel_bridge`, which on some startup
        // paths runs before `register_domain_subscribers` initialises
        // the bus. A one-shot check would silently no-op for the rest
        // of the process; a short polling loop with a hard cap retries
        // without spinning forever if init genuinely never happens
        // (e.g. tests that drive the socket layer in isolation).
        let bus = {
            const RETRY_INTERVAL_MS: u64 = 250;
            const MAX_WAIT_SECS: u64 = 30;
            let max_attempts = (MAX_WAIT_SECS * 1000) / RETRY_INTERVAL_MS;
            let mut attempts: u64 = 0;
            loop {
                if let Some(bus) = crate::core::event_bus::global() {
                    break bus;
                }
                attempts += 1;
                if attempts > max_attempts {
                    log::warn!(
                        "[socketio] event_bus not initialised after {}s — SessionExpired bridge giving up",
                        MAX_WAIT_SECS
                    );
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(RETRY_INTERVAL_MS)).await;
            }
        };
        let mut rx = bus.raw_receiver();
        loop {
            let event = match rx.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!(
                        "[socketio] dropped {} event_bus events due to lag (auth bridge)",
                        skipped
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            if let crate::core::event_bus::DomainEvent::SessionExpired { source, reason } = event {
                log::info!(
                    "[socketio] broadcast auth:session_expired source={} reason_len={}",
                    source,
                    reason.len()
                );
                // The UI doesn't need the raw reason (already logged
                // server-side and we don't want auth-error strings in the
                // renderer console). Just send the source slug.
                let payload = serde_json::json!({ "source": source });
                let _ = io_auth.emit("auth:session_expired", &payload);
                let _ = io_auth.emit("auth_session_expired", &payload);
            }
        }
        log::debug!("[socketio] auth session_expired bridge stopped");
    });

    // 5. Transcription results → broadcast to all connected clients.
    tokio::spawn(async move {
        let mut rx = crate::openhuman::voice::dictation_listener::subscribe_transcription_results();
        loop {
            let text = match rx.recv().await {
                Ok(text) => text,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!(
                        "[socketio] dropped {} transcription events due to lag",
                        skipped
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };

            log::debug!(
                "[socketio] broadcast dictation:transcription ({} chars) to all clients",
                text.len()
            );
            let payload = serde_json::json!({ "text": text });
            let _ = io_transcription.emit("dictation:transcription", &payload);
        }
        log::debug!("[socketio] transcription bridge stopped");
    });

    // 7. Companion state change events → broadcast to all clients so the
    //    overlay and settings panel can react to session lifecycle and
    //    state transitions (Idle → Listening → Thinking → Speaking → …).
    tokio::spawn(async move {
        let mut rx = crate::openhuman::desktop_companion::bus::subscribe_state_changed();
        loop {
            let event = match rx.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!(
                        "[socketio] dropped {} companion state_changed events due to lag",
                        skipped
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };

            if let Ok(payload) = serde_json::to_value(&event) {
                log::debug!(
                    "[socketio] broadcast companion:state_changed session={} {} -> {}",
                    event.session_id,
                    event.previous_state,
                    event.state,
                );
                let _ = io_companion.emit("companion:state_changed", &payload);
                let _ = io_companion.emit("companion_state_changed", &payload);
            }
        }
        log::debug!("[socketio] companion state bridge stopped");
    });
}

/// Join `socket` to `room`, logging the result.
///
/// `socket.join()` returns a `Result` that historically was discarded
/// with `let _ = …`. Silent failure on the `"system"` room in
/// particular makes proactive-message delivery vanish without a trace,
/// so both the happy and error paths are logged with enough context
/// (room name + client id) to diagnose missing welcome messages from
/// logs alone.
fn join_room_logged(socket: &SocketRef, room: &str, client_id: &str) {
    match socket.join(room.to_string()) {
        Ok(()) => log::debug!("[socketio] joined room '{room}' for client {client_id}"),
        Err(e) => log::warn!("[socketio] failed to join room '{room}' for client {client_id}: {e}"),
    }
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

#[cfg(test)]
mod tests {
    use super::{event_alias, origin_is_allowed};

    #[test]
    fn event_alias_translates_between_delimiters() {
        assert_eq!(event_alias("chat_done").as_deref(), Some("chat:done"));
        assert_eq!(event_alias("chat:error").as_deref(), Some("chat_error"));
        assert_eq!(event_alias("ready"), None);
    }

    #[test]
    fn origin_allowlist_accepts_native_clients() {
        assert!(origin_is_allowed(None));
    }

    #[test]
    fn origin_allowlist_accepts_tauri_localhost_across_schemes() {
        // The CEF-served app shell stamps a platform-dependent Origin:
        //   - macOS / iOS use the native `tauri://localhost` scheme
        //   - Windows uses the CEF custom HTTP protocol → `http://tauri.localhost`
        //   - Linux / older Windows builds use `https://tauri.localhost`
        // All three flavours are the same trust tier (the bundled webview),
        // so each must pass the handshake gate.
        assert!(origin_is_allowed(Some("tauri://localhost")));
        assert!(origin_is_allowed(Some("https://tauri.localhost")));
        assert!(origin_is_allowed(Some("http://tauri.localhost")));
    }

    #[test]
    fn origin_allowlist_accepts_local_dev_server() {
        assert!(origin_is_allowed(Some("http://localhost:1420")));
        assert!(origin_is_allowed(Some("http://127.0.0.1:1420")));
        assert!(origin_is_allowed(Some("http://[::1]:1420")));
        // Loopback without an explicit port (some CEF builds stamp this
        // shape when the shell runs on the default port).
        assert!(origin_is_allowed(Some("http://localhost")));
    }

    #[test]
    fn origin_allowlist_rejects_cross_origin_browser_pages() {
        assert!(!origin_is_allowed(Some("https://attacker.example")));
        assert!(!origin_is_allowed(Some("http://evil.local")));
        assert!(!origin_is_allowed(Some("null")));
        assert!(!origin_is_allowed(Some("")));
    }

    #[test]
    fn origin_allowlist_rejects_host_prefix_decoys() {
        // Regression: `starts_with("localhost")` accepted these; the exact
        // host match must not.
        assert!(!origin_is_allowed(Some(
            "http://localhost.attacker.example"
        )));
        assert!(!origin_is_allowed(Some(
            "http://127.0.0.1.attacker.example"
        )));
        assert!(!origin_is_allowed(Some("https://localhost-evil")));
        // Same rule applies to the tauri.localhost host — must be exact.
        assert!(!origin_is_allowed(Some(
            "http://tauri.localhost.attacker.example"
        )));
        assert!(!origin_is_allowed(Some("https://tauri.localhost.evil")));
    }

    #[test]
    fn origin_allowlist_rejects_unparseable_origin() {
        assert!(!origin_is_allowed(Some("not a url")));
        assert!(!origin_is_allowed(Some("javascript:alert(1)")));
    }
}
