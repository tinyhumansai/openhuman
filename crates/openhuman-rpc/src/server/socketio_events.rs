//! Socket.IO connect authorization and the per-socket event handlers.
//!
//! Both run in the namespace's connect middleware. socketioxide 0.18 sends the
//! namespace CONNECT packet after the middleware and before the connect
//! handler, and drops an event that arrives for a handler not yet registered.
//! Registered in the connect handler, a client's first event (the app sends
//! `thread:subscribe` as soon as it sees CONNECT) could land in that gap and
//! never be answered.

use super::*;

/// Authorizes a connecting socket and, when it passes, marks it authenticated
/// and registers its event handlers. A refused socket gets neither; the
/// connect handler disconnects it.
pub(super) fn authorize_and_register(
    socket: &SocketRef,
    token: Option<String>,
    preflight_gate: Option<super::super::auth::PreflightGate>,
) {
    if authorize(socket, token, &preflight_gate) {
        register_event_handlers(socket, preflight_gate);
    }
}

fn authorize(
    socket: &SocketRef,
    token: Option<String>,
    handshake_gate: &Option<super::super::auth::PreflightGate>,
) -> bool {
    let client_id = socket.id.to_string();
        if handshake_gate.as_ref().is_some_and(|gate| gate.is_active()) {
            log::info!("[socketio] rejecting connect during startup preflight client={client_id}");
            return false;
        }

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
            return false;
        }

        // Verify the handshake bearer matches the per-process RPC token.
        // `TryData` lets us treat a missing/malformed `auth` payload as a
        // soft failure (no panic) and reject the connect cleanly.
        let supplied = token.unwrap_or_default();
        if !crate::core_host::core::auth::verify_bearer_token(&supplied) {
            log::warn!(
                "[socketio] rejecting connect: missing or invalid bearer client={}",
                client_id
            );
            return false;
        }
        socket.extensions.insert(AuthedConnection);
        let Some(tenant) = socket_tenant_from_request_extensions(
            &socket.req_parts().extensions,
        ) else {
            log::warn!("[socketio] rejecting socket without a tenant scope in SaaS mode");
            return false;
        };
        socket.extensions.insert(tenant);
    true
}

fn register_event_handlers(
    socket: &SocketRef,
    preflight_gate: Option<super::super::auth::PreflightGate>,
) {
    // Handler for JSON-RPC over WebSocket.
    let rpc_gate = preflight_gate.clone();
    socket.on(
        "rpc:request",
        move |socket: SocketRef, Data(payload): Data<SocketRpcRequest>| {
            let rpc_gate = rpc_gate.clone();
            async move {
            if rpc_gate.as_ref().is_some_and(|gate| gate.is_active()) {
                let _ = socket.emit(
                    "rpc:error",
                    &json!({
                        "id": payload.id,
                        "error": {
                            "code": -32003,
                            "message": "RPC is unavailable during startup approval preflight"
                        }
                    }),
                );
                return;
            }
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
                // A Socket.IO event is not scoped to a request, so it
                // runs under the process default context.
                crate::core_host::core::runtime::DispatchCtx::process_default(),
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
            }
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
                        crate::core_host::agent::tinyagents::run_mode::set_mode(
                            &crate::core_host::core::runtime::DispatchCtx::host_default(),
                            &thread_id,
                            mode,
                        );
                    }
                    None => log::warn!(
                        "[socketio] chat:start thread_id={thread_id} ignoring unrecognized run_mode={run_mode}"
                    ),
                }
            }

            // Trigger the web channel's chat logic.
            match crate::core_host::web_chat::start_chat(
                // Outside SaaS the host's one context; in SaaS the socket task's own scope.
                &crate::core_host::core::runtime::DispatchCtx::host_default(),
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
                &crate::core_host::core::runtime::DispatchCtx::host_default(),
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
            // The ack is sent only after the join, so a client that
            // reads on the strength of it is already in the room.
            join_room_logged(&socket, &room, &socket.id.to_string());
            // Hand this socket whatever the approval gate still has
            // parked on the thread, BEFORE acknowledging the join, so a
            // client that orders its recovery reads against the ack
            // already holds the card.
            replay_parked_approval(&socket, thread_id);
            replay_parked_plan_review(&socket, thread_id);
            ack.send(&ThreadSubscribeAck { joined: true }).ok();
        },
    );
}
