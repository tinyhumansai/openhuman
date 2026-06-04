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
    /// Structured rate-limit / error metadata produced by
    /// `classify_inference_error` (issue #2606). All four fields are
    /// additive — older FE clients that only read `message`/`error_type`
    /// keep working; new clients can read these to render countdown,
    /// retry-button, and fallback-CTA UI without regexing the message.
    ///
    /// Where the limit originated:
    /// `"provider"` | `"openhuman_budget"` | `"agent_loop"`
    /// | `"openhuman_billing"` | `"transport"` | `"config"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_source: Option<String>,
    /// Whether the same prompt can be retried in this same thread.
    /// `false` for non-retryable business 429s, auth, model_unavailable,
    /// context_overflow, and billing exhaustion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_retryable: Option<bool>,
    /// Milliseconds to wait before retrying, as supplied by the upstream
    /// `Retry-After:` / `retry_after:` header. `None` when the upstream
    /// didn't supply one or the error class has no retry-after concept.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_retry_after_ms: Option<u64>,
    /// Provider name extracted from `"<provider> API error (...)"`
    /// envelopes. `None` for non-provider errors (OpenHuman budget cap,
    /// agent loop) and for transport failures without a provider prefix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_provider: Option<String>,
    /// `Some(false)` once the reliable-provider chain has exhausted
    /// every configured `model_fallbacks` entry. `None` means "unknown
    /// — FE should not promise a fallback".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_fallback_available: Option<bool>,
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
    /// Persistent worker sub-thread id backing the delegation (on
    /// `subagent_spawned`). The frontend stores it on the subagent row and
    /// uses it to reopen the full parent↔subagent conversation from memory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_thread_id: Option<String>,
    /// Human-readable display name from the agent registry (e.g.
    /// "Researcher", "Coding Agent"). The frontend uses this for
    /// consistent agent labels across timeline, sub-mascots, and drawer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
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
    #[serde(default)]
    queue_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatCancelPayload {
    thread_id: String,
}

#[derive(Debug, Deserialize)]
struct ThreadSubscribePayload {
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
                        payload.queue_mode,
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

            // Handler for subscribing this socket to a thread's room.
            //
            // Chat-stream events are delivered to BOTH the initiating client's
            // own room AND a per-thread room (`thread:<id>`). After a socket
            // reconnects it has a NEW client_id, so it would miss an in-flight
            // turn's remaining stream (delivered to the OLD client_id room). The
            // frontend emits this on connect/reconnect for the active thread, so
            // the new socket re-joins the thread room and keeps receiving the
            // stream. Membership is dropped automatically on disconnect.
            socket.on(
                "thread:subscribe",
                |socket: SocketRef, Data(payload): Data<ThreadSubscribePayload>| async move {
                    if !socket_is_authed(&socket) {
                        drop_unauthed(&socket, "thread:subscribe from unauthenticated socket");
                        return;
                    }
                    let thread_id = payload.thread_id.trim();
                    if thread_id.is_empty() {
                        return;
                    }
                    let room = format!("thread:{thread_id}");
                    join_room_logged(&socket, &room, &socket.id.to_string());
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
    let io_mcp_setup = io.clone();
    let io_memory_sync = io.clone();
    let io_agent_meetings = io.clone();

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

    // 6b. McpSetupSecretRequested → broadcast `mcp_setup:secret_requested`
    //     so the UI can render a native input dialog. Only the opaque
    //     ref + safe display fields are forwarded; raw secret values
    //     are not part of the event payload.
    tokio::spawn(async move {
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
                        "[socketio] event_bus not initialised after {}s — mcp_setup bridge giving up",
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
                        "[socketio] dropped {} event_bus events due to lag (mcp_setup bridge)",
                        skipped
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            if let crate::core::event_bus::DomainEvent::McpSetupSecretRequested {
                ref_id,
                key_name,
                prompt,
            } = event
            {
                log::info!(
                    "[socketio] broadcast mcp_setup:secret_requested ref={} key={}",
                    ref_id,
                    key_name
                );
                let payload = serde_json::json!({
                    "ref_id": ref_id,
                    "key_name": key_name,
                    "prompt": prompt,
                });
                let _ = io_mcp_setup.emit("mcp_setup:secret_requested", &payload);
                let _ = io_mcp_setup.emit("mcp_setup_secret_requested", &payload);
            }
        }
        log::debug!("[socketio] mcp_setup secret_requested bridge stopped");
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

    // 8. Memory sync stage + tree-build progress → broadcast to all clients
    //    so the UI can show real-time progress bars and refresh the graph.
    tokio::spawn(async move {
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
                        "[socketio] event_bus not initialised after {}s — memory_sync bridge giving up",
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
                        "[socketio] dropped {} event_bus events due to lag (memory_sync bridge)",
                        skipped
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            match event {
                crate::core::event_bus::DomainEvent::MemorySyncStageChanged {
                    trigger,
                    stage,
                    provider,
                    connection_id,
                    detail,
                } => {
                    let payload = serde_json::json!({
                        "trigger": trigger,
                        "stage": stage,
                        "provider": provider,
                        "connection_id": connection_id,
                        "detail": detail,
                    });
                    let _ = io_memory_sync.emit("memory:sync_stage", &payload);
                }
                crate::core::event_bus::DomainEvent::TreeSummarizerPropagated {
                    namespace,
                    node_id,
                    level,
                    token_count,
                } => {
                    let payload = serde_json::json!({
                        "namespace": namespace,
                        "node_id": node_id,
                        "level": level,
                        "token_count": token_count,
                    });
                    let _ = io_memory_sync.emit("memory:tree_progress", &payload);
                }
                crate::core::event_bus::DomainEvent::TreeSummarizerRebuildCompleted {
                    namespace,
                    total_nodes,
                } => {
                    let payload = serde_json::json!({
                        "namespace": namespace,
                        "total_nodes": total_nodes,
                    });
                    let _ = io_memory_sync.emit("memory:tree_completed", &payload);
                }
                crate::core::event_bus::DomainEvent::MemoryTreeBuildProgress {
                    phase,
                    step,
                    tree_scope,
                    level,
                    item_count,
                    detail,
                } => {
                    let payload = serde_json::json!({
                        "phase": phase,
                        "step": step,
                        "tree_scope": tree_scope,
                        "level": level,
                        "item_count": item_count,
                        "detail": detail,
                    });
                    let _ = io_memory_sync.emit("memory:build_progress", &payload);
                }
                _ => {}
            }
        }
        log::debug!("[socketio] memory_sync bridge stopped");
    });

    // 9. Backend Meet bot events → broadcast to all connected frontend sockets.
    tokio::spawn(async move {
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
                        "[socketio] event_bus not initialised after {}s — agent_meetings bridge giving up",
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
                        "[socketio] dropped {} event_bus events due to lag (agent_meetings bridge)",
                        skipped
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            match event {
                crate::core::event_bus::DomainEvent::BackendMeetJoined { meet_url } => {
                    let payload = serde_json::json!({ "meet_url": meet_url });
                    log::debug!("[socketio] broadcast agent_meetings:joined");
                    let _ = io_agent_meetings.emit("agent_meetings:joined", &payload);
                }
                crate::core::event_bus::DomainEvent::BackendMeetLeft { reason } => {
                    let payload = serde_json::json!({ "reason": reason });
                    log::debug!("[socketio] broadcast agent_meetings:left reason={}", reason);
                    let _ = io_agent_meetings.emit("agent_meetings:left", &payload);
                }
                crate::core::event_bus::DomainEvent::BackendMeetReply {
                    transcript,
                    reply,
                    emotion,
                } => {
                    let payload = serde_json::json!({
                        "transcript": transcript,
                        "reply": reply,
                        "emotion": emotion,
                    });
                    log::debug!(
                        "[socketio] broadcast agent_meetings:reply reply_len={}",
                        reply.len()
                    );
                    let _ = io_agent_meetings.emit("agent_meetings:reply", &payload);
                }
                crate::core::event_bus::DomainEvent::BackendMeetHarness {
                    transcript,
                    instruction,
                    emotion,
                } => {
                    let payload = serde_json::json!({
                        "transcript": transcript,
                        "instruction": instruction,
                        "emotion": emotion,
                    });
                    log::debug!(
                        "[socketio] broadcast agent_meetings:harness instruction_len={}",
                        instruction.len()
                    );
                    let _ = io_agent_meetings.emit("agent_meetings:harness", &payload);
                }
                crate::core::event_bus::DomainEvent::BackendMeetTranscript {
                    turns,
                    duration_ms,
                } => {
                    let payload = serde_json::json!({
                        "turns": turns,
                        "duration_ms": duration_ms,
                    });
                    log::debug!(
                        "[socketio] broadcast agent_meetings:transcript turns={} duration_ms={}",
                        turns.len(),
                        duration_ms
                    );
                    let _ = io_agent_meetings.emit("agent_meetings:transcript", &payload);
                }
                crate::core::event_bus::DomainEvent::BackendMeetError { error } => {
                    let payload = serde_json::json!({ "error": error });
                    log::debug!("[socketio] broadcast agent_meetings:error");
                    let _ = io_agent_meetings.emit("agent_meetings:error", &payload);
                }
                _ => {}
            }
        }
        log::debug!("[socketio] agent_meetings bridge stopped");
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
    let name = event.event.clone();
    // Deliver to the initiating client's own room AND the per-thread room. The
    // thread room lets a socket that reconnected with a new client_id (after
    // re-subscribing via `thread:subscribe`) keep receiving an in-flight turn's
    // stream.
    //
    // ⚠️ socketioxide (0.15.2) does NOT de-duplicate a socket present in
    // multiple target rooms: `LocalAdapter::apply_opts` flattens each room's
    // sid-set and collects WITHOUT a dedup pass, so `io.to([a, b]).emit()`
    // delivers TWICE to a socket in both `a` and `b`. The initiating client is
    // in both its `client_id` room and the `thread:<id>` room it subscribed to
    // → every streamed frame doubled ("double thinking"). So we emit to the
    // `client_id` room, then to the thread room EXCEPT the `client_id` room —
    // each socket is reached exactly once regardless of room overlap.
    // "system" broadcasts and events without a thread_id keep single-room delivery.
    let primary = event.client_id.clone();
    let thread_room = (event.client_id != "system" && !event.thread_id.is_empty())
        .then(|| format!("thread:{}", event.thread_id));
    if let Ok(payload) = serde_json::to_value(event) {
        log::debug!(
            "[socketio] send event={} primary={} thread_room={:?} thread_id={} request_id={}",
            name,
            primary,
            thread_room,
            payload
                .get("thread_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default(),
            payload
                .get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
        );
        // Primary: the client_id room.
        let _ = io.to(primary.clone()).emit(&name, &payload);
        if let Some(alias) = event_alias(&name) {
            let _ = io.to(primary.clone()).emit(alias, &payload);
        }
        // Thread room minus the client_id room (dedup — see note above).
        if let Some(tr) = thread_room {
            let _ = io
                .to(tr.clone())
                .except(primary.clone())
                .emit(&name, &payload);
            if let Some(alias) = event_alias(&name) {
                let _ = io
                    .to(tr.clone())
                    .except(primary.clone())
                    .emit(alias, &payload);
            }
        }
    }
}

/// Events that stream once per token (their payloads concatenate into the final
/// text / thinking / tool-args). Emitting the legacy `:`-delimited alias for
/// these doubles every frame on the wire — the "double thinking-token
/// streaming" bug — and no client subscribes to the colon variant, so the alias
/// is suppressed for exactly these. Enumerated explicitly rather than matched by
/// a `*_delta` suffix, so a future *discrete* event whose name happens to end in
/// `_delta` still gets its compat alias instead of being silently dropped.
const STREAMING_DELTA_EVENTS: &[&str] = &["text_delta", "thinking_delta", "tool_args_delta"];

fn event_alias(name: &str) -> Option<String> {
    // Match against the canonical underscore form after stripping a `subagent_`
    // prefix (subagent streaming mirrors the parent's deltas), so `text_delta`,
    // `text:delta`, and `subagent_text_delta` all resolve to a listed event.
    // Lower-frequency discrete events keep the compat alias.
    let normalized = name.replace(':', "_");
    let base = normalized.strip_prefix("subagent_").unwrap_or(&normalized);
    if STREAMING_DELTA_EVENTS.contains(&base) {
        return None;
    }
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
    fn event_alias_suppressed_for_streaming_deltas() {
        // Streaming deltas must NOT be aliased — doubling every token frame is
        // the "double thinking-token streaming" bug. Discrete events still alias.
        assert_eq!(event_alias("thinking_delta"), None);
        assert_eq!(event_alias("text_delta"), None);
        assert_eq!(event_alias("tool_args_delta"), None);
        assert_eq!(event_alias("subagent_tool_args_delta"), None);
        // A *discrete* event that merely ends in `_delta` is NOT a streaming
        // token event and must keep its compat alias — this is what the explicit
        // STREAMING_DELTA_EVENTS set guarantees over the old `*_delta` suffix.
        assert_eq!(
            event_alias("inventory_delta").as_deref(),
            Some("inventory:delta")
        );
        // Sanity: a non-delta event in the same family still aliases.
        assert_eq!(event_alias("tool_call").as_deref(), Some("tool:call"));
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
