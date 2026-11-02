use super::{
    channel_connection_update_payload, emit_web_channel_event, event_alias,
    origin_is_allowed_with_extra, publish_companion_state_changed, should_broadcast_session_expiry,
    socket_tenant_from_request_extensions, stamp_replay_timestamp,
    subscribe_companion_state_changed, ThreadSubscribePayload,
};

use crate::core_host::web_chat::{WebChannelEvent, WebEventContext};
use engineioxide::Packet;
use socketioxide::{extract::SocketRef, SocketIo};

#[tokio::test]
async fn socket_connect_reads_dispatch_context_from_request_extensions_after_spawn() {
    let dispatch = crate::core_host::core::runtime::DispatchCtx::process_default();
    let expected = crate::core_host::agent::plan_review::socket_scope_for_dispatch(&dispatch);
    let mut extensions = axum::http::Extensions::new();
    extensions.insert(dispatch);

    // Mirrors socketioxide's async connect handler boundary: request parts
    // retain Axum extensions, while the callback itself runs in a new task.
    let actual = tokio::spawn(async move {
        socket_tenant_from_request_extensions(&extensions).map(|tenant| tenant.0)
    })
    .await
    .expect("spawned connect callback completes");

    assert_eq!(actual, expected);
}

#[test]
fn thread_subscribe_cannot_choose_a_profile_for_review_replay() {
    let payload = serde_json::from_value::<ThreadSubscribePayload>(serde_json::json!({
        "thread_id": "thread-1",
        "profile_id": "victim-profile",
    }));
    assert!(
        payload.is_err(),
        "tenant selection is not a client payload field"
    );
}

#[test]
fn session_expiry_bridge_only_broadcasts_for_the_current_session_credential() {
    let old_request = crate::core_host::credentials::backend_credential::credential_fingerprint(
        "expired-request-token",
    );
    assert!(!should_broadcast_session_expiry(
        Some(&old_request),
        Some("replacement-token"),
        false,
    ));
    assert!(should_broadcast_session_expiry(
        Some(&old_request),
        Some("expired-request-token"),
        false,
    ));
    assert!(should_broadcast_session_expiry(
        Some(&old_request),
        None,
        false,
    ));
    assert!(!should_broadcast_session_expiry(
        None,
        Some("current-token"),
        false,
    ));
    assert!(!should_broadcast_session_expiry(
        Some(&old_request),
        Some("expired-request-token"),
        true,
    ));
}

#[test]
fn companion_state_transport_delivers_payload_to_bridge() {
    let mut rx = subscribe_companion_state_changed();
    let payload = serde_json::json!({
        "session_id": "session-1",
        "state": "thinking",
        "previous_state": "listening",
    });
    assert!(publish_companion_state_changed(payload.clone()) >= 1);
    assert_eq!(rx.try_recv().expect("payload delivered"), payload);
}

#[test]
fn replay_timestamp_is_stored_in_the_typed_event_context() {
    let mut event = crate::core_host::web_chat::approval_request_event(
        "request-1",
        "read_file",
        "inspect the document",
        &serde_json::json!({}),
        "thread-1",
        "client-1",
        None,
        None,
        false,
    );

    stamp_replay_timestamp(&mut event);

    assert!(event.context().ts.is_some());
}

#[test]
fn channel_connection_update_payload_connected_omits_error() {
    let payload = channel_connection_update_payload("discord", "connected", None);
    assert_eq!(payload["channel"], "discord");
    // Listener-backed channels always map to the bot_token auth mode.
    assert_eq!(payload["auth_mode"], "bot_token");
    assert_eq!(payload["status"], "connected");
    assert!(
        payload.get("last_error").is_none(),
        "connected payload must not carry a last_error: {payload}"
    );
}

#[test]
fn channel_connection_update_payload_error_carries_reason() {
    let payload =
        channel_connection_update_payload("discord", "error", Some("gateway closed (4004)"));
    assert_eq!(payload["channel"], "discord");
    assert_eq!(payload["auth_mode"], "bot_token");
    assert_eq!(payload["status"], "error");
    assert_eq!(payload["last_error"], "gateway closed (4004)");
}

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

#[tokio::test]
async fn web_events_reach_client_and_thread_rooms_once_with_compat_aliases() {
    let (_service, io) = SocketIo::new_svc();
    let request_id = format!(
        "socketio-routing-{}",
        crate::core_host::web_chat::unix_epoch_ms()
    );
    let (connected_tx, mut connected_rx) = tokio::sync::mpsc::unbounded_channel();
    io.ns("/", move |socket: SocketRef| {
        connected_tx.send(socket).expect("test receiver stays open");
    });

    let (_initiator_tx, mut initiator_rx) = io.new_dummy_sock("/", ()).await;
    receive_connect(&mut initiator_rx).await;
    let initiator = connected_rx.recv().await.expect("initiator socket");
    initiator.join("client-a").expect("join client room");
    initiator.join("thread:thread-1").expect("join thread room");

    let (_reconnected_tx, mut reconnected_rx) = io.new_dummy_sock("/", ()).await;
    receive_connect(&mut reconnected_rx).await;
    let reconnected = connected_rx.recv().await.expect("reconnected peer socket");
    reconnected
        .join("thread:thread-1")
        .expect("reconnected peer joins thread room");

    let (_unrelated_tx, mut unrelated_rx) = io.new_dummy_sock("/", ()).await;
    receive_connect(&mut unrelated_rx).await;
    let unrelated = connected_rx.recv().await.expect("unrelated peer socket");
    unrelated
        .join("thread:thread-2")
        .expect("unrelated peer joins another thread");

    let done = next_scoped_event(WebChannelEvent::ChatDone {
        context: WebEventContext {
            client_id: "client-a".into(),
            thread_id: "thread-1".into(),
            request_id: request_id.clone(),
            ..Default::default()
        },
        full_response: Some("finished".into()),
        citations: None,
        usage: None,
        timing: None,
        parts: None,
    })
    .await;
    emit_web_channel_event(&io, done);

    let initiator_packets = receive_messages(&mut initiator_rx, 2).await;
    let reconnected_packets = receive_messages(&mut reconnected_rx, 2).await;
    assert_eq!(event_names(&initiator_packets), ["chat_done", "chat:done"]);
    assert_eq!(
        event_names(&reconnected_packets),
        ["chat_done", "chat:done"]
    );
    assert!(initiator_packets
        .iter()
        .all(|(_, payload)| payload["event"] == "chat_done"));
    assert!(reconnected_packets
        .iter()
        .all(|(_, payload)| payload["event"] == "chat_done"));
    assert!(initiator_packets
        .iter()
        .all(|(_, payload)| payload["full_response"] == "finished"));
    assert!(reconnected_packets
        .iter()
        .all(|(_, payload)| payload["thread_id"] == "thread-1"));
    assert_empty_queue(&mut initiator_rx);
    assert_empty_queue(&mut reconnected_rx);
    assert_empty_queue(&mut unrelated_rx);

    let delta = next_scoped_event(WebChannelEvent::TextDelta {
        context: WebEventContext {
            client_id: "client-a".into(),
            thread_id: "thread-1".into(),
            request_id,
            ..Default::default()
        },
        delta: Some("token".into()),
        delta_kind: None,
        round: None,
    })
    .await;
    emit_web_channel_event(&io, delta);

    let initiator_delta = receive_messages(&mut initiator_rx, 1).await;
    let reconnected_delta = receive_messages(&mut reconnected_rx, 1).await;
    assert_eq!(event_names(&initiator_delta), ["text_delta"]);
    assert_eq!(event_names(&reconnected_delta), ["text_delta"]);
    assert_eq!(initiator_delta[0].1["event"], "text_delta");
    assert_eq!(reconnected_delta[0].1["event"], "text_delta");
    assert_eq!(initiator_delta[0].1["delta"], "token");
    assert_eq!(reconnected_delta[0].1["delta"], "token");
    assert_empty_queue(&mut initiator_rx);
    assert_empty_queue(&mut reconnected_rx);
    assert_empty_queue(&mut unrelated_rx);
}

async fn next_scoped_event(event: WebChannelEvent) -> crate::core_host::web_chat::ScopedWebEvent {
    let expected = event.context().clone();
    let expected_name = event.event_name();
    let mut events = crate::core_host::web_chat::subscribe_web_channel_events();
    crate::core_host::web_chat::publish_web_channel_event(event);
    tokio::time::timeout(std::time::Duration::from_secs(1), async move {
        loop {
            match events.recv().await {
                Ok(event)
                    if event.context().client_id == expected.client_id
                        && event.context().thread_id == expected.thread_id
                        && event.context().request_id == expected.request_id
                        && event.event_name() == expected_name =>
                {
                    return event;
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    panic!("web event bus closed before publishing test event")
                }
            }
        }
    })
    .await
    .expect("published web event reaches test subscriber")
}

async fn receive_messages(
    rx: &mut tokio::sync::mpsc::Receiver<Packet>,
    count: usize,
) -> Vec<(String, serde_json::Value)> {
    let mut messages = Vec::with_capacity(count);
    while messages.len() < count {
        let packet = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("expected event packet before timeout")
            .expect("socket stays connected");
        if let Packet::Message(message) = packet {
            let payload = message
                .strip_prefix('2')
                .expect("Engine.IO message packet starts with packet type 2");
            let packet: Vec<serde_json::Value> =
                serde_json::from_str(payload).unwrap_or_else(|error| {
                    panic!("invalid Socket.IO event packet {message:?}: {error}")
                });
            assert_eq!(packet.len(), 2, "event packet has name and payload");
            messages.push((
                packet[0]
                    .as_str()
                    .expect("Socket.IO packet has event name")
                    .to_string(),
                packet[1].clone(),
            ));
        }
    }
    messages
}

async fn receive_connect(rx: &mut tokio::sync::mpsc::Receiver<Packet>) {
    tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
        .await
        .expect("socket handshake arrives before timeout")
        .expect("socket stays connected during handshake");
}

fn event_names(packets: &[(String, serde_json::Value)]) -> Vec<&str> {
    packets.iter().map(|(name, _)| name.as_str()).collect()
}

fn assert_empty_queue(rx: &mut tokio::sync::mpsc::Receiver<Packet>) {
    assert!(
        matches!(
            rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ),
        "socket received an unexpected duplicate event"
    );
}

#[test]
fn origin_allowlist_accepts_native_clients() {
    assert!(origin_is_allowed_with_extra(None, None));
}

#[test]
fn origin_allowlist_accepts_tauri_localhost_across_schemes() {
    // The CEF-served app shell stamps a platform-dependent Origin:
    //   - macOS / iOS use the native `tauri://localhost` scheme
    //   - Windows uses the CEF custom HTTP protocol → `http://tauri.localhost`
    //   - Linux / older Windows builds use `https://tauri.localhost`
    // All three flavours are the same trust tier (the bundled webview),
    // so each must pass the handshake gate.
    assert!(origin_is_allowed_with_extra(
        Some("tauri://localhost"),
        None
    ));
    assert!(origin_is_allowed_with_extra(
        Some("https://tauri.localhost"),
        None
    ));
    assert!(origin_is_allowed_with_extra(
        Some("http://tauri.localhost"),
        None
    ));
}

#[test]
fn origin_allowlist_accepts_local_dev_server() {
    assert!(origin_is_allowed_with_extra(
        Some("http://localhost:1420"),
        None
    ));
    assert!(origin_is_allowed_with_extra(
        Some("http://127.0.0.1:1420"),
        None
    ));
    assert!(origin_is_allowed_with_extra(
        Some("http://[::1]:1420"),
        None
    ));
    // Loopback without an explicit port (some CEF builds stamp this
    // shape when the shell runs on the default port).
    assert!(origin_is_allowed_with_extra(Some("http://localhost"), None));
}

#[test]
fn origin_allowlist_rejects_cross_origin_browser_pages() {
    assert!(!origin_is_allowed_with_extra(
        Some("https://attacker.example"),
        None
    ));
    assert!(!origin_is_allowed_with_extra(
        Some("http://evil.local"),
        None
    ));
    assert!(!origin_is_allowed_with_extra(Some("null"), None));
    assert!(!origin_is_allowed_with_extra(Some(""), None));
}

#[test]
fn origin_allowlist_rejects_host_prefix_decoys() {
    // Regression: `starts_with("localhost")` accepted these; the exact
    // host match must not.
    assert!(!origin_is_allowed_with_extra(
        Some("http://localhost.attacker.example"),
        None
    ));
    assert!(!origin_is_allowed_with_extra(
        Some("http://127.0.0.1.attacker.example"),
        None
    ));
    assert!(!origin_is_allowed_with_extra(
        Some("https://localhost-evil"),
        None
    ));
    // Same rule applies to the tauri.localhost host — must be exact.
    assert!(!origin_is_allowed_with_extra(
        Some("http://tauri.localhost.attacker.example"),
        None
    ));
    assert!(!origin_is_allowed_with_extra(
        Some("https://tauri.localhost.evil"),
        None
    ));
}

#[test]
fn origin_allowlist_accepts_env_allowlisted_origin() {
    // Regression: a non-loopback browser origin that the JSON-RPC CORS
    // layer accepts must also pass the socket handshake, or chat (a
    // socket-only transport) silently never connects.
    let extra = Some("https://box.example.ts.net:9000");
    assert!(origin_is_allowed_with_extra(
        Some("https://box.example.ts.net:9000"),
        extra
    ));
}

#[test]
fn origin_allowlist_env_handles_comma_list_and_whitespace() {
    let extra = Some("https://a.example:9000 , https://b.example:8443");
    assert!(origin_is_allowed_with_extra(
        Some("https://a.example:9000"),
        extra
    ));
    assert!(origin_is_allowed_with_extra(
        Some("https://b.example:8443"),
        extra
    ));
    assert!(!origin_is_allowed_with_extra(
        Some("https://c.example"),
        extra
    ));
}

#[test]
fn origin_allowlist_env_requires_exact_match() {
    // Populating the allowlist must not loosen the decoy rules: no port
    // wildcarding, no scheme swapping, no suffix matching.
    let extra = Some("https://box.example.ts.net:9000");
    assert!(!origin_is_allowed_with_extra(
        Some("http://box.example.ts.net:9000"),
        extra
    ));
    assert!(!origin_is_allowed_with_extra(
        Some("https://box.example.ts.net:9001"),
        extra
    ));
    assert!(!origin_is_allowed_with_extra(
        Some("https://box.example.ts.net.attacker.example:9000"),
        extra
    ));
}

#[test]
fn origin_allowlist_env_does_not_rescue_null_or_empty() {
    let extra = Some("null,");
    assert!(!origin_is_allowed_with_extra(Some("null"), extra));
    assert!(!origin_is_allowed_with_extra(Some(""), extra));
}

#[test]
fn origin_allowlist_rejects_unparseable_origin() {
    assert!(!origin_is_allowed_with_extra(Some("not a url"), None));
    assert!(!origin_is_allowed_with_extra(
        Some("javascript:alert(1)"),
        None
    ));
}

/// Install the per-process RPC bearer once (it is a process-wide `OnceLock`
/// that sibling tests may already have seeded) and return the live value.
fn ensure_rpc_token() -> String {
    // Idempotent: a no-op when a sibling test already seeded the bearer.
    crate::core_host::core::auth::init_rpc_token_with_value("socketio-handshake-tests-token")
        .expect("seed rpc bearer");
    crate::core_host::core::auth::get_rpc_token()
        .expect("rpc bearer installed")
        .to_string()
}

/// Drive one Engine.IO v4 long-polling session: open, send the Socket.IO
/// CONNECT with `auth`, then poll until the server either emits `ready` or
/// disconnects the namespace. Returns everything the server sent.
async fn polling_handshake(base: &str, auth: serde_json::Value) -> String {
    let http = reqwest::Client::new();
    let open = http
        .get(format!("{base}/socket.io/?EIO=4&transport=polling"))
        .send()
        .await
        .expect("engine.io open")
        .text()
        .await
        .unwrap();
    let open: serde_json::Value =
        serde_json::from_str(open.trim_start_matches('0')).expect("engine.io open packet");
    let sid = open["sid"].as_str().expect("engine.io sid").to_owned();
    let url = format!("{base}/socket.io/?EIO=4&transport=polling&sid={sid}");
    http.post(&url)
        .body(format!("40{auth}"))
        .send()
        .await
        .expect("socket.io connect");
    let mut received = String::new();
    for _ in 0..3 {
        let batch = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            http.get(&url).send().await.ok()?.text().await.ok()
        })
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
        received.push_str(&batch);
        received.push('\u{1e}');
        if received.contains("\"ready\"") || received.split('\u{1e}').any(|p| p == "41") {
            break;
        }
    }
    received
}

/// H5: the handshake bearer gate, end to end over a real socket. Only the
/// per-process core bearer gets `ready`; a missing or wrong token is
/// disconnected before any event reaches the client.
#[tokio::test]
async fn only_the_core_bearer_completes_the_socketio_handshake() {
    // Loopback-only fixture; bounded so a stalled poll fails fast instead of
    // waiting out every retry.
    tokio::time::timeout(std::time::Duration::from_secs(30), handshake_bearer_gate())
        .await
        .expect("socket.io handshake test timed out");
}

#[tokio::test]
async fn startup_preflight_denies_authenticated_socketio_until_open_and_guards_rpc_events() {
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        startup_preflight_socket_gate(),
    )
    .await
    .expect("Socket.IO preflight test timed out");
}

async fn startup_preflight_socket_gate() {
    let token = ensure_rpc_token();
    let gate = crate::server::auth::PreflightGate::new_active();
    let router = crate::server::http::build_core_http_router_with_preflight(true, gate.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let client = reqwest::Client::new();
    let denied = client
        .get(format!("{base}/socket.io/?EIO=4&transport=polling"))
        .send()
        .await
        .expect("preflight Engine.IO open");
    assert!(
        denied.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "initial Engine.IO request must be denied before allocating a session: {}",
        denied.status()
    );
    let denial = denied.text().await.unwrap();
    assert!(
        denial.contains("startup_preflight"),
        "preflight body: {denial}"
    );

    gate.open();
    let open = client
        .get(format!("{base}/socket.io/?EIO=4&transport=polling"))
        .send()
        .await
        .expect("engine.io open after gate")
        .text()
        .await
        .unwrap();
    let open: serde_json::Value =
        serde_json::from_str(open.trim_start_matches('0')).expect("engine.io open packet");
    let sid = open["sid"].as_str().expect("engine.io sid").to_owned();
    let url = format!("{base}/socket.io/?EIO=4&transport=polling&sid={sid}");
    client
        .post(&url)
        .body(format!(
            "40{}",
            serde_json::json!({ "token": ensure_rpc_token() })
        ))
        .send()
        .await
        .expect("authenticated socket connect after gate");
    let connected = client.get(&url).send().await.unwrap().text().await.unwrap();
    assert!(
        connected.contains("\"ready\""),
        "post-preflight ready: {connected}"
    );

    // A live authenticated socket must still check the same gate before
    // dispatching RPC events. Re-closing is test-only; production opens once.
    gate.close_for_test();
    client
        .post(&url)
        .body(
            "42[\"rpc:request\",{\"id\":\"probe\",\"method\":\"openhuman.config_get\",\"params\":{}}]",
        )
        .send()
        .await
        .expect("send authenticated RPC event");
    let denied_rpc = client.get(&url).send().await.unwrap().text().await.unwrap();
    assert!(
        denied_rpc.contains("rpc:error"),
        "preflight rpc response: {denied_rpc}"
    );
    assert!(
        denied_rpc.contains("unavailable during startup approval preflight"),
        "preflight must not dispatch the protected method: {denied_rpc}"
    );
}

#[test]
fn keyring_approval_socket_payload_contains_only_operation_metadata() {
    let request = crate::core_host::core::events::KeyringAccessApprovalRequest {
        request_id: "request-1".into(),
        caller_agent: "openhuman.credentials".into(),
        origin: "LocalOperator".into(),
        purpose: "OpenHuman auth profile credential access".into(),
        access: "write".into(),
        name: "user-1:auth:openai:default".into(),
        expires_at_ms: 1_800_000_000_000,
    };
    let payload = super::keyring_access_approval_payload(&request);
    assert_eq!(payload["request_id"], "request-1");
    assert_eq!(payload["access"], "write");
    assert_eq!(payload["name"], "user-1:auth:openai:default");
    assert!(payload.get("value").is_none());
}

async fn handshake_bearer_gate() {
    let token = ensure_rpc_token();
    let (layer, _io) = super::attach_socketio();
    // The serving router stamps every request with its dispatch context; the
    // connect handler refuses a socket that arrives without one.
    let app = axum::Router::new().layer(layer).layer(axum::Extension(
        crate::core_host::core::runtime::DispatchCtx::host_default(),
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    for (auth, label) in [
        (
            serde_json::json!({ "token": "not-the-core-token" }),
            "wrong",
        ),
        (serde_json::json!({}), "absent"),
    ] {
        let received = polling_handshake(&base, auth).await;
        assert!(!received.contains("\"ready\""), "{label}: {received}");
        assert!(
            received.split('\u{1e}').any(|packet| packet == "41"),
            "{label} token must be disconnected: {received}"
        );
    }

    let received = polling_handshake(&base, serde_json::json!({ "token": token })).await;
    assert!(received.contains("\"ready\""), "valid token: {received}");
}
