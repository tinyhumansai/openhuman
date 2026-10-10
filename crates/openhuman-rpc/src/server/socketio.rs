//! Socket.IO live-event bridge to the desktop shell.
//!
//! `spawn_web_channel_bridge` spawns one forwarding task per source. Domain
//! broadcast channels: web-chat events (`openhuman::web_chat`), dictation hotkeys
//! and transcription results (`openhuman::voice::dictation_listener`), overlay
//! attention bubbles (`openhuman::desktop::overlay::subscribe_attention_events`,
//! see `desktop/overlay/README.md`), core notifications
//! (`openhuman::desktop::notifications`), and shell companion state
//! (`COMPANION_STATE_BUS`). `DomainEvent`s read off `openhuman::core::bus::BUS`:
//! session expiry, MCP setup secret requests, memory sync and tree-build
//! progress, channel listener health, and active-workspace changes. Web-chat
//! events go to the initiating client's room and the `thread:<id>` room
//! (`emit_web_channel_event`); everything else is broadcast to every
//! connected client, most under both a colon- and an underscore-separated
//! event name for frontend compatibility. `COMPANION_STATE_BUS` is a broadcast
//! channel dedicated to shell-originated companion lifecycle events: the
//! companion implementation itself lives in the Tauri shell, but the native
//! macOS notch WKWebView has no Tauri IPC bridge and connects to the
//! embedded core's Socket.IO endpoint directly, so this module keeps a
//! transport-only seam for it rather than reintroducing a core-side
//! companion domain.
//!
//! The socketioxide/axum transport bodies (the actual `SocketIo` server,
//! connection handlers, and `spawn_web_channel_bridge`) are gated behind the
//! `http-server` feature (#5048). The event payload types further down
//! (`WebChannelEvent`, `TurnUsagePayload`, `SubagentUsagePayload`,
//! `SubagentProgressDetail`) stay compiled in every build regardless — around
//! ten always-on domains (`web_chat`, `cron`, `channels`, `agent`, …)
//! construct them — so only the transport surface is gated, not the types
//! (a "type carve-out"; see AGENTS.md). `pub mod socketio;` in `core::mod` is
//! intentionally NOT gated for the same reason.

use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use serde_json::Value;
use socketioxide::extract::{AckSender, Data, SocketRef, TryData};
use socketioxide::SocketIo;

use crate::core_host::web_chat::{GuardrailPayload, WebChannelEvent};

/// Shell-originated companion lifecycle events that still need to reach
/// Socket.IO-only surfaces such as the native macOS notch WKWebView.
static COMPANION_STATE_BUS: once_cell::sync::Lazy<tokio::sync::broadcast::Sender<Value>> =
    once_cell::sync::Lazy::new(|| {
        let (tx, _rx) = tokio::sync::broadcast::channel(64);
        tx
    });

/// Publish a shell-side companion state payload for Socket.IO clients.
///
/// The companion implementation lives in the Tauri shell, but the notch
/// WKWebView has no Tauri IPC bridge and connects directly to the embedded
/// core's Socket.IO endpoint. Keeping this transport-only seam here avoids
/// reintroducing the removed core companion domain.
pub fn publish_companion_state_changed(payload: Value) -> usize {
    COMPANION_STATE_BUS.send(payload).unwrap_or_default()
}

fn subscribe_companion_state_changed() -> tokio::sync::broadcast::Receiver<Value> {
    COMPANION_STATE_BUS.subscribe()
}

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
pub(crate) fn origin_is_allowed(origin: Option<&str>) -> bool {
    origin_is_allowed_with_extra(
        origin,
        std::env::var(crate::ALLOWED_ORIGINS_ENV).ok().as_deref(),
    )
}

/// Origin gate with the extra allowlist passed explicitly so tests do not have
/// to mutate process-global env.
///
/// Chat is a socket-only transport (`chat:start` / `chat:cancel`) with no
/// JSON-RPC equivalent. Before this consulted `OPENHUMAN_CORE_ALLOWED_ORIGINS`,
/// a non-loopback browser origin that the RPC CORS layer already accepted was
/// still dropped here: the page loaded and every read RPC succeeded, but the
/// socket was disconnected at handshake and pressing Send did nothing, with no
/// error surfaced to the user.
pub(crate) fn origin_is_allowed_with_extra(
    origin: Option<&str>,
    extra_origins: Option<&str>,
) -> bool {
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
    if matches!(
        parsed.host_str(),
        Some("localhost" | "127.0.0.1" | "::1" | "[::1]" | "tauri.localhost")
    ) {
        return true;
    }

    // Operator-controlled extra origins. Exact string match, same rule as the
    // JSON-RPC layer — no host-only or prefix matching, so the decoy cases
    // below stay rejected even when the allowlist is populated.
    if let Some(extra) = extra_origins {
        for candidate in extra.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if candidate == origin {
                return true;
            }
        }
    }

    false
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
    locale: Option<String>,
    #[serde(default)]
    queue_mode: Option<String>,
    /// Optional `"plan"` | `"build"` — lets the composer start this turn with
    /// the thread already in the requested run mode (e.g. a "Plan" toggle),
    /// rather than a separate `agent.set_run_mode` round-trip racing the
    /// `chat:start` itself. Unrecognized values are ignored (logged), not
    /// rejected — a stale/typo'd client build should not fail the whole turn.
    #[serde(default)]
    run_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatCancelPayload {
    thread_id: String,
    /// The request this cancel targets. When the client passes the id of the
    /// turn it started, the cancel is scoped to that turn so a late cancel for a
    /// timed-out request can't kill the next turn on the thread (#4760).
    #[serde(default)]
    request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThreadSubscribePayload {
    thread_id: String,
}

/// Reply to `thread:subscribe`, so a client can order a read after the join.
#[derive(Debug, Serialize)]
struct ThreadSubscribeAck {
    joined: bool,
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
            if !crate::core_host::core::auth::verify_bearer_token(&supplied) {
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
            let _ = join_room_logged(&socket, &client_id, &client_id);
            // Also auto-join the "system" room so every connected client
            // receives broadcast-style events that aren't tied to a
            // specific chat thread. Today this covers proactive messages
            // (welcome agent, morning briefing, cron-driven announcements)
            // which `channels::proactive::ProactiveMessageSubscriber`
            // emits with `client_id = "system"` — see `emit_web_channel_event`.
            // If this join fails the welcome message silently disappears,
            // so we log both success and failure for diagnosability.
            let _ = join_room_logged(&socket, "system", &client_id);
            let ready_payload = json!({ "sid": client_id });
            log::debug!("[socketio] emit event=ready to_client={}", socket.id);
            let _ = socket.emit("ready", &ready_payload);

            // Seed this client with the workspace that is current (#5966).
            // The `workspace_changed` bridge in `spawn_web_channel_bridge`
            // only fires on a switch, so a client that connects between
            // switches — the common case, since the app connects at launch —
            // would otherwise have no idea which workspace is active and
            // could not scope anything.
            //
            // Spawned because this handler is synchronous and the resolve is
            // not. Emitting to `socket` rather than broadcasting keeps a
            // late-joining client from re-announcing a workspace every other
            // client already knows about.
            {
                let socket = socket.clone();
                let client_id = client_id.clone();
                tokio::spawn(async move {
                    match crate::core_host::config::active_workspace_snapshot().await {
                        Ok((dir, revision)) => {
                            let handle = crate::core_host::config::workspace_handle(&dir);
                            // One snapshot, not two reads: resolved
                            // separately, a switch between them would pair
                            // this workspace with the *next* one's revision,
                            // and the client would rank a stale seed above
                            // the switch it lost to. This task and the switch
                            // bridge are separate, so that race is real; the
                            // client keeps the highest revision it has seen.
                            log::debug!(
                                "[socketio] emit event=workspace_changed to_client={client_id} workspace={handle} revision={revision}"
                            );
                            let payload =
                                json!({ "workspace": handle, "revision": revision });
                            let _ = socket.emit("workspace_changed", &payload);
                            let _ = socket.emit("workspace:changed", &payload);
                        }
                        Err(error) => log::warn!(
                            "[socketio] could not resolve the active workspace to seed client={client_id}: {error}"
                        ),
                    }
                });
            }

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
                    let response = match crate::core_host::core::invoke::invoke_method(
                        crate::core_host::core::invoke::default_state(),
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
                    if let Some(run_mode) = payload.run_mode.as_deref() {
                        match crate::core_host::agent::tinyagents::run_mode::parse_mode_label(run_mode) {
                            Some(mode) => {
                                crate::core_host::agent::tinyagents::run_mode::set_mode(&thread_id, mode);
                            }
                            None => log::warn!(
                                "[socketio] chat:start thread_id={thread_id} ignoring unrecognized run_mode={run_mode}"
                            ),
                        }
                    }

                    // Trigger the web channel's chat logic.
                    match crate::core_host::web_chat::start_chat(
                        &client_id,
                        &payload.thread_id,
                        &payload.message,
                        model_override,
                        payload.temperature,
                        payload.locale,
                        payload.queue_mode,
                        crate::core_host::web_chat::ChatRequestMetadata::default(),
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
                            let mut error_payload = json!({
                                "event": "chat_error",
                                "client_id": client_id,
                                "thread_id": thread_id,
                                "request_id": "",
                                "message": error.to_string(),
                                "error_type": "inference",
                            });
                            // A guardrail rejection is structured (verdict/
                            // score/reasons), not just a user-facing message —
                            // surface it the same way the frontend classifies
                            // every other `chat_error`: by `error_type`, plus
                            // a typed `guardrail` payload it doesn't have to
                            // parse out of `message`.
                            if let crate::core_host::web_chat::StartChatError::Guardrail {
                                verdict,
                                score,
                                reasons,
                            } = &error
                            {
                                error_payload["error_type"] = json!("guardrail");
                                error_payload["guardrail"] = json!(GuardrailPayload {
                                    verdict: verdict.clone(),
                                    score: *score,
                                    reasons: reasons.clone(),
                                });
                            }
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
                    let _ = crate::core_host::web_chat::cancel_chat_scoped(
                        &client_id,
                        &payload.thread_id,
                        payload.request_id.as_deref(),
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
            //
            // The join is acknowledged so a client can *order* work against it.
            // A reconnecting client re-reads the thread to pick up a reply that
            // landed while it was away (#6034); firing that read before the join
            // is processed leaves a window where the read misses the row and the
            // turn's `chat_done` is emitted to a room this socket has not joined
            // yet, so the reply stays invisible until a manual reload. The ack
            // closes it. Clients that ignore the ack are unaffected — an unused
            // acknowledgement is inert.
            socket.on(
                "thread:subscribe",
                |socket: SocketRef, Data(payload): Data<ThreadSubscribePayload>, ack: AckSender| async move {
                    if !socket_is_authed(&socket) {
                        drop_unauthed(&socket, "thread:subscribe from unauthenticated socket");
                        return;
                    }
                    let thread_id = payload.thread_id.trim();
                    if thread_id.is_empty() {
                        // Still acknowledge: a client awaiting this must not be
                        // left hanging on its own malformed payload.
                        ack.send(&ThreadSubscribeAck { joined: false }).ok();
                        return;
                    }
                    let room = format!("thread:{thread_id}");
                    // Report what actually happened. Acknowledging a join that
                    // failed is worse than not acknowledging at all: the client
                    // stops queueing the thread for retry and reads on the
                    // strength of a room it is not in.
                    let joined = join_room_logged(&socket, &room, &socket.id.to_string());
                    // Hand this socket whatever the approval gate still has
                    // parked on the thread, BEFORE acknowledging the join, so a
                    // client that orders its recovery reads against the ack
                    // already holds the card.
                    if joined {
                        replay_parked_approval(&socket, thread_id);
                        replay_parked_plan_review(&socket, thread_id);
                    }
                    ack.send(&ThreadSubscribeAck { joined }).ok();
                },
            );
        },
    );

    (layer, io)
}

/// Spawns background bridges to forward various system events to Socket.IO clients.
///
/// This function sets up event bridges:
/// 1. **Web Channel Bridge**: Forwards chat-related events (messages, tool calls) to specific clients.
/// 2. **Dictation Bridge**: Forwards hotkey events to all clients.
/// 3. **Overlay Bridge**: Forwards attention bubble events to all clients.
/// 4. **Core Notification Bridge**: Forwards core notification events to all clients.
/// 5. **Transcription Bridge**: Forwards real-time speech-to-text results to all clients.
pub fn spawn_web_channel_bridge(io: SocketIo) {
    // 1. Web channel events → per-client rooms.
    let io_web = io.clone();
    tokio::spawn(async move {
        let mut rx = crate::core_host::web_chat::subscribe_web_channel_events();
        loop {
            let event = match rx.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!("[socketio] dropped {skipped} web channel events due to lag");
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
    let io_memory_sync = io.clone();
    let io_channel_status = io.clone();
    let io_companion = io.clone();
    let io_workspace = io.clone();

    // 2. Dictation hotkey events → broadcast to all connected clients.
    tokio::spawn(async move {
        let mut rx = crate::core_host::voice::dictation_listener::subscribe_dictation_events();
        loop {
            let event = match rx.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!("[socketio] dropped {skipped} events due to lag");
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

    // Shell companion state → broadcast to all clients. The main renderer also
    // receives a Tauri event directly; this path preserves the Socket.IO-only
    // native notch surface.
    tokio::spawn(async move {
        let mut rx = subscribe_companion_state_changed();
        loop {
            let payload = match rx.recv().await {
                Ok(payload) => payload,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!(
                        "[socketio] dropped {} companion state_changed events due to lag",
                        skipped
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            log::debug!("[socketio] broadcast companion:state_changed");
            let _ = io_companion.emit("companion:state_changed", &payload);
            let _ = io_companion.emit("companion_state_changed", &payload);
        }
        log::debug!("[socketio] companion state bridge stopped");
    });

    // 3. Overlay attention events → broadcast to all clients.
    tokio::spawn(async move {
        let mut rx = crate::core_host::desktop::overlay::subscribe_attention_events();
        loop {
            let event = match rx.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!("[socketio] dropped {skipped} events due to lag");
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
        let mut rx = crate::core_host::desktop::notifications::subscribe_core_notifications();
        loop {
            let event = match rx.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!("[socketio] dropped {skipped} events due to lag");
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
                if let Some(bus) = crate::core_host::core::bus::BUS.get() {
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
        let mut rx = bus.receiver();
        loop {
            let Some(event) = rx.recv().await else {
                break;
            };
            if let crate::core_host::core::events::DomainEvent::SessionExpired { source, reason } =
                event
            {
                // Other publishers may emit a backend 401 while this core is
                // using its offline local credential. The auth subscriber
                // correctly keeps that credential, so the UI must not receive
                // a contradictory sign-out event from this independent bus
                // consumer. Real JWT expiry still broadcasts as before.
                if crate::core_host::security::credentials::session_support::current_session_is_local(
                )
                .await
                {
                    log::info!(
                        "[socketio] suppress auth:session_expired for local offline credential source={}",
                        source
                    );
                    continue;
                }
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

    // 6a. ActiveWorkspaceChanged → broadcast `workspace_changed` carrying the
    //     new workspace's opaque handle (#5966).
    //
    //     `core_notification` is emitted to every connected client with no
    //     per-client routing, and the publish-time gate that decides whether a
    //     workspace-bound notification may be broadcast resolves the active
    //     workspace and then sends — two steps, not one. A switch in between
    //     still lets one through. Telling clients the handle of the workspace
    //     that is current lets the receiver re-check on render instead of
    //     trusting a boolean taken at an instant.
    //
    //     The handle, never `workspace_dir`: this reaches every connected
    //     client and the path is under the user's home directory.
    tokio::spawn(async move {
        let bus = {
            const RETRY_INTERVAL_MS: u64 = 250;
            const MAX_WAIT_SECS: u64 = 30;
            let max_attempts = (MAX_WAIT_SECS * 1000) / RETRY_INTERVAL_MS;
            let mut attempts: u64 = 0;
            loop {
                if let Some(bus) = crate::core_host::core::bus::BUS.get() {
                    break bus;
                }
                attempts += 1;
                if attempts > max_attempts {
                    log::warn!(
                        "[socketio] event_bus not initialised after {}s — workspace bridge giving up",
                        MAX_WAIT_SECS
                    );
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(RETRY_INTERVAL_MS)).await;
            }
        };
        let mut rx = bus.receiver();
        loop {
            let Some(event) = rx.recv().await else {
                break;
            };
            if let crate::core_host::core::events::DomainEvent::ActiveWorkspaceChanged {
                workspace_dir,
                revision,
            } = event
            {
                let handle = crate::core_host::config::workspace_handle(&workspace_dir);
                log::info!(
                    "[socketio] broadcast workspace_changed workspace={handle} revision={revision}"
                );
                let payload = serde_json::json!({ "workspace": handle, "revision": revision });
                let _ = io_workspace.emit("workspace_changed", &payload);
                let _ = io_workspace.emit("workspace:changed", &payload);
            }
        }
        log::debug!("[socketio] workspace_changed bridge stopped");
    });

    // 5. Transcription results → broadcast to all connected clients.
    tokio::spawn(async move {
        let mut rx = crate::core_host::voice::dictation_listener::subscribe_transcription_results();
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

    // 8. Memory sync stage + tree-build progress → broadcast to all clients
    //    so the UI can show real-time progress bars and refresh the graph.
    tokio::spawn(async move {
        let bus = {
            const RETRY_INTERVAL_MS: u64 = 250;
            const MAX_WAIT_SECS: u64 = 30;
            let max_attempts = (MAX_WAIT_SECS * 1000) / RETRY_INTERVAL_MS;
            let mut attempts: u64 = 0;
            loop {
                if let Some(bus) = crate::core_host::core::bus::BUS.get() {
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
        let mut rx = bus.receiver();
        loop {
            let Some(event) = rx.recv().await else {
                break;
            };
            match event {
                // Live per-step progress of an in-flight flow run (issue G2).
                // Best-effort: the durable `flow_runs` row is the source of
                // truth and the Workflows UI keeps a 2s poller as fallback, so
                // a dropped event here (broadcast lag) only delays the live
                // update, never corrupts run history.
                crate::core_host::core::events::DomainEvent::FlowRunProgress {
                    run_id,
                    node_id,
                    status,
                } => {
                    let payload = serde_json::json!({
                        "run_id": run_id,
                        "node_id": node_id,
                        "status": status,
                    });
                    log::debug!(
                        "[socketio] broadcast flow_run_progress run_id={} node_id={} status={}",
                        run_id,
                        node_id,
                        status
                    );
                    let _ = io_memory_sync.emit("flow:run_progress", &payload);
                    let _ = io_memory_sync.emit("flow_run_progress", &payload);
                }
                // A `flow_runs` row was just persisted, before execution begins
                // (issue B35, runs-rail live refresh). Broadcast so an open
                // Workflows canvas/sidebar can show "Running" immediately
                // instead of waiting for the blocking `flows_run` RPC to
                // resolve or the first `FlowRunProgress` step. Best-effort,
                // same rationale as `flow:run_progress` above.
                crate::core_host::core::events::DomainEvent::FlowRunStarted { flow_id, run_id } => {
                    let payload = serde_json::json!({
                        "flow_id": flow_id,
                        "run_id": run_id,
                    });
                    log::debug!(
                        "[socketio] broadcast flow_run_started flow_id={} run_id={}",
                        flow_id,
                        run_id
                    );
                    let _ = io_memory_sync.emit("flow:run_started", &payload);
                    let _ = io_memory_sync.emit("flow_run_started", &payload);
                }
                // The terminal companion to `FlowRunStarted` above (issue B35
                // follow-up). Published once `flows::ops::finish_flow_run_row`
                // persists the settled `flow_runs` row, so an open Workflows
                // canvas/sidebar can flip a run to Completed/Failed live
                // instead of relying on a poll to notice. Best-effort, same
                // rationale as the other `flow:*` bridges.
                crate::core_host::core::events::DomainEvent::FlowRunFinished {
                    flow_id,
                    run_id,
                    status,
                } => {
                    let payload = serde_json::json!({
                        "flow_id": flow_id,
                        "run_id": run_id,
                        "status": status,
                    });
                    log::debug!(
                        "[socketio] broadcast flow_run_finished flow_id={} run_id={} status={}",
                        flow_id,
                        run_id,
                        status
                    );
                    let _ = io_memory_sync.emit("flow:run_finished", &payload);
                    let _ = io_memory_sync.emit("flow_run_finished", &payload);
                }
                // A saved flow's definition changed (create/update/delete/
                // enable). Broadcast so an open Workflows list/canvas refetches
                // — most importantly, so an agent `save_workflow` becomes
                // visible in a canvas the user has open (audit F6). Best-effort;
                // the UI's refetch-on-focus is the backstop.
                crate::core_host::core::events::DomainEvent::FlowChanged {
                    flow_id,
                    kind,
                    actor,
                } => {
                    let payload = serde_json::json!({
                        "flow_id": flow_id,
                        "kind": kind,
                        "actor": actor,
                    });
                    log::debug!(
                        "[socketio] broadcast flow_changed flow_id={} kind={} actor={}",
                        flow_id,
                        kind,
                        actor
                    );
                    let _ = io_memory_sync.emit("flow:changed", &payload);
                    let _ = io_memory_sync.emit("flow_changed", &payload);
                }
                // A Workflow-origin tool call parked in the `ApprovalGate`
                // (flow-approval-surface, PR2/PR3). Broadcast — not
                // room-scoped like `ApprovalRequested`'s `approval_request`
                // bridge — because a flow run has no chat thread/client to
                // target; the Workflows UI listens process-wide and filters
                // by `flow_id`/`run_id` client-side.
                crate::core_host::core::events::DomainEvent::FlowApprovalRequested {
                    request_id,
                    flow_id,
                    run_id,
                    tool_name,
                    summary,
                    agent_id: _,
                } => {
                    let payload = serde_json::json!({
                        "request_id": request_id,
                        "flow_id": flow_id,
                        "run_id": run_id,
                        "tool_name": tool_name,
                        "summary": summary,
                    });
                    log::info!(
                        "[socketio] broadcast flow_approval_request request_id={} flow_id={} run_id={} tool={}",
                        request_id,
                        flow_id,
                        run_id,
                        tool_name
                    );
                    let _ = io_memory_sync.emit("flow_approval_request", &payload);
                }
                _ => {}
            }
        }
        log::debug!("[socketio] memory_sync bridge stopped");
    });

    // 10. Channel listener health → broadcast `channel:connection-updated` to
    //     all clients so the Messaging tab reflects the *live* connection state
    //     instead of a stale, credential-presence-only "Connected" (issue
    //     #3712). The supervised listener publishes `ChannelConnected` when it
    //     (re)enters its recv loop and `ChannelDisconnected { reason }` when it
    //     errors/exits. Only listener-backed channels (telegram/discord
    //     `bot_token`) fire these, so we map them to the `bot_token` auth mode —
    //     the frontend `normalizeChannelConnectionUpdatePayload` drops any
    //     channel/mode it doesn't recognise.
    tokio::spawn(async move {
        let bus = {
            const RETRY_INTERVAL_MS: u64 = 250;
            const MAX_WAIT_SECS: u64 = 30;
            let max_attempts = (MAX_WAIT_SECS * 1000) / RETRY_INTERVAL_MS;
            let mut attempts: u64 = 0;
            loop {
                if let Some(bus) = crate::core_host::core::bus::BUS.get() {
                    break bus;
                }
                attempts += 1;
                if attempts > max_attempts {
                    log::warn!(
                        "[socketio] event_bus not initialised after {}s — channel_status bridge giving up",
                        MAX_WAIT_SECS
                    );
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(RETRY_INTERVAL_MS)).await;
            }
        };
        let mut rx = bus.receiver();
        loop {
            let Some(event) = rx.recv().await else {
                break;
            };
            let payload = match event {
                crate::core_host::core::events::DomainEvent::ChannelConnected { channel } => {
                    log::debug!(
                        "[socketio] broadcast channel:connection-updated {channel} -> connected"
                    );
                    Some(channel_connection_update_payload(
                        &channel,
                        "connected",
                        None,
                    ))
                }
                crate::core_host::core::events::DomainEvent::ChannelDisconnected {
                    channel,
                    reason,
                } => {
                    log::debug!(
                        "[socketio] broadcast channel:connection-updated {channel} -> error reason_len={}",
                        reason.len()
                    );
                    Some(channel_connection_update_payload(
                        &channel,
                        "error",
                        Some(&reason),
                    ))
                }
                _ => None,
            };
            if let Some(payload) = payload {
                // Emit both colon and underscore variants for FE compatibility.
                let _ = io_channel_status.emit("channel:connection-updated", &payload);
                let _ = io_channel_status.emit("channel_connection_updated", &payload);
            }
        }
        log::debug!("[socketio] channel_status bridge stopped");
    });
}

/// Build the `channel:connection-updated` payload broadcast when a supervised
/// channel listener changes state (issue #3712). Listener-backed channels are
/// always the `bot_token` auth mode (the only mode that materialises a runtime
/// listener for telegram/discord); `last_error` carries the disconnect reason.
/// Matches the shape consumed by the frontend
/// `normalizeChannelConnectionUpdatePayload`.
pub(crate) fn channel_connection_update_payload(
    channel: &str,
    status: &str,
    last_error: Option<&str>,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "channel": channel,
        "auth_mode": "bot_token",
        "status": status,
    });
    if let Some(reason) = last_error {
        payload["last_error"] = serde_json::Value::String(reason.to_string());
    }
    payload
}

/// Join `socket` to `room`, logging the result.
///
/// `socket.join()` returns a `Result` that historically was discarded
/// with `let _ = …`. Silent failure on the `"system"` room in
/// particular makes proactive-message delivery vanish without a trace,
/// so both the happy and error paths are logged with enough context
/// (room name + client id) to diagnose missing welcome messages from
/// logs alone.
///
/// Returns whether the socket is actually in the room. Callers that only log
/// may ignore it; a caller that *tells the client* it joined must not — a
/// client told it is in a room it never joined reads the thread, waits for
/// events that will never be routed to it, and reproduces the invisible-reply
/// bug this room exists to prevent (#6034).
fn join_room_logged(socket: &SocketRef, room: &str, client_id: &str) -> bool {
    match socket.join(room.to_string()) {
        Ok(()) => {
            log::debug!("[socketio] joined room '{room}' for client {client_id}");
            true
        }
        Err(e) => {
            log::warn!("[socketio] failed to join room '{room}' for client {client_id}: {e}");
            false
        }
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

/// Re-send the approval parked on `thread_id`, if any, to the socket that just
/// joined that thread's room.
///
/// An approval is durable server-side state — the gate holds the parked call
/// and a `pending_approvals` row — but it reaches the UI as ONE fire-and-forget
/// emit from [`emit_web_channel_event`]. That emit can miss with no error and
/// no trace: `io.to(room).emit()` on a room whose only member has gone is a
/// silent no-op, there is no disconnect handler here so the core never learns a
/// client died, a socket that reconnects lands in the thread room only for
/// events emitted *after* it joins, and the bridge drops frames wholesale on
/// broadcast lag. Any one of those leaves the turn parked forever with no card
/// on screen and no way for the user to act.
///
/// `thread:subscribe` is the one signal that says "this socket is now watching
/// this thread", which makes it the place to reconcile the two. Replaying is
/// safe to repeat: the client keys the card by `request_id` and a decided
/// request is no longer parked, so a socket that already has the card just
/// re-renders the same one.
///
/// Every park on the thread is replayed, oldest first: several async
/// sub-agents can each be waiting on the same parent thread, and replaying
/// only the newest left the others unanswerable until they expired.
fn replay_parked_approval(socket: &SocketRef, thread_id: &str) {
    let Some(gate) = crate::core_host::security::approval::ApprovalGate::try_global() else {
        return;
    };
    let client_id = socket.id.to_string();
    for row in gate.parked_requests_for_thread(thread_id) {
        let expires_at = row.expires_at.map(|t| t.to_rfc3339());
        let mut event = crate::core_host::web_chat::approval_request_event(
            &row.request_id,
            &row.tool_name,
            &row.action_summary,
            &row.args_redacted,
            thread_id,
            &client_id,
            row.tool_call_id.as_deref(),
            expires_at.as_deref(),
            gate.request_is_detached(&row.request_id),
        );
        // Replay is a fresh emit to a newly-joined socket, not a resend of the
        // original event, so stamp `ts` with "now" (same clock as
        // `publish_web_channel_event`) rather than leaving it unset.
        event.ts = Some(crate::core_host::web_chat::unix_epoch_ms());
        let Ok(payload) = serde_json::to_value(&event) else {
            continue;
        };
        log::info!(
            "[socketio] replaying parked approval_request to joining socket client_id={client_id} thread_id={thread_id} request_id={} tool={}",
            row.request_id,
            row.tool_name
        );
        emit_with_aliases(socket, "approval_request", &payload);
    }
}

/// Re-send the plan review parked on `thread_id`, if any, to the socket that
/// just joined that thread's room. Mirrors [`replay_parked_approval`] — a
/// plan review is a live, in-memory park (no SQLite row), but it reaches the
/// UI the same fire-and-forget way, so the same reconciliation applies.
fn replay_parked_plan_review(socket: &SocketRef, thread_id: &str) {
    let Some(row) =
        crate::core_host::agent::plan_review::gate::global().parked_review_for_thread(thread_id)
    else {
        return;
    };
    let client_id = socket.id.to_string();
    let mut event = crate::core_host::web_chat::plan_review_request_event(
        &row.request_id,
        &row.summary,
        &row.steps,
        thread_id,
        &client_id,
        row.tool_call_id.as_deref(),
        row.expires_at.as_deref(),
    );
    event.ts = Some(crate::core_host::web_chat::unix_epoch_ms());
    let Ok(payload) = serde_json::to_value(&event) else {
        return;
    };
    log::info!(
        "[socketio] replaying parked plan_review_request to joining socket client_id={client_id} thread_id={thread_id} request_id={}",
        row.request_id
    );
    emit_with_aliases(socket, "plan_review_request", &payload);
}

fn emit_with_aliases(socket: &SocketRef, name: &str, payload: &serde_json::Value) {
    let _ = socket.emit(name, payload);
    if let Some(alias) = event_alias(name) {
        let _ = socket.emit(alias, payload);
    }
}

// Every test here names a gated fn (`channel_connection_update_payload`,
// `event_alias`, `origin_is_allowed`), so the module gates in lockstep (#5048).
#[cfg(test)]
#[path = "socketio_tests.rs"]
mod tests;
