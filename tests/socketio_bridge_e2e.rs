//! Socket.IO end to end: an authenticated websocket client receives every
//! live event the core's bridges forward, and a socket that is in both its own
//! room and a thread room receives each web-channel frame exactly once.
//!
//! The broadcast calls behind these bridges return futures since socketioxide
//! 0.16; a future that is dropped instead of awaited compiles and sends nothing,
//! so this test reads the frames off a real connection rather than checking
//! that the calls were made.
//!
//! Its own binary because it boots a core and owns the process event bus (see
//! the runtime-affinity note on `openhuman_core::core::bus::init`).

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use openhuman_core::config::Config;
use openhuman_core::core::bus::BUS;
use openhuman_core::core::events::DomainEvent;
use openhuman_core::core::runtime::{CoreBuilder, DomainSet, ServiceSet, TokenSource};
use openhuman_core::desktop::notifications::{
    publish_core_notification, CoreNotificationCategory, CoreNotificationEvent,
};
use openhuman_core::web_chat::{publish_web_channel_event, WebChannelEvent};
use openhuman_core::HostKind;
use openhuman_rpc::server::{build_core_http_router, publish_companion_state_changed};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

#[path = "support/tinyhumans_boot.rs"]
mod tinyhumans_boot;

const TOKEN: &str = "socketio-bridge-e2e-token";

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The next non-ping Engine.IO frame. Pings (`2`) are answered (`3`) so the
/// server keeps the connection open however long the test runs.
async fn next_frame(ws: &mut Ws) -> String {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("a frame within 10s")
            .expect("connection open")
            .expect("websocket frame");
        let Message::Text(text) = message else {
            continue;
        };
        if text.as_str() == "2" {
            ws.send(Message::text("3")).await.expect("pong");
            continue;
        }
        return text.to_string();
    }
}

/// `42["name", payload]` as `(name, payload)`.
fn event_of(frame: &str) -> Option<(String, Value)> {
    let array: Vec<Value> = serde_json::from_str(frame.strip_prefix("42")?).ok()?;
    let name = array.first()?.as_str()?.to_string();
    Some((name, array.get(1).cloned().unwrap_or(Value::Null)))
}

/// Reads frames until the event `name` arrives and returns its payload.
async fn wait_for(ws: &mut Ws, name: &str) -> Value {
    loop {
        if let Some((event, payload)) = event_of(&next_frame(ws).await) {
            if event == name {
                return payload;
            }
        }
    }
}

/// Publishes until the event `name` arrives. Each bridge subscribes from its
/// own spawned task, so an event published before that task runs is missed;
/// re-publishing makes the test independent of when that happens.
async fn publish_until(ws: &mut Ws, name: &str, publish: impl Fn()) -> Value {
    for _ in 0..80 {
        publish();
        if let Ok(payload) =
            tokio::time::timeout(Duration::from_millis(250), wait_for(ws, name)).await
        {
            return payload;
        }
    }
    panic!("`{name}` never reached the client");
}

/// Connects and authenticates, returning as soon as the namespace CONNECT ack
/// arrives. That is the point where the app's `connect` handler starts
/// sending events, `thread:subscribe` included, without waiting for `ready`.
async fn connect(addr: std::net::SocketAddr) -> Ws {
    let url = format!("ws://{addr}/socket.io/?EIO=4&transport=websocket");
    let (mut ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("websocket connect");
    let open = next_frame(&mut ws).await;
    assert!(open.starts_with('0'), "engine.io open packet, got {open}");
    ws.send(Message::text(format!("40{}", json!({ "token": TOKEN }))))
        .await
        .expect("namespace connect");
    loop {
        if next_frame(&mut ws).await.starts_with("40") {
            return ws;
        }
    }
}

/// Event names received until `end` arrives, in order, `end` excluded.
async fn events_until(ws: &mut Ws, end: &str) -> Vec<String> {
    let mut seen = Vec::new();
    loop {
        if let Some((name, _)) = event_of(&next_frame(ws).await) {
            if name == end {
                return seen;
            }
            seen.push(name);
        }
    }
}

/// How many times `name` appears in `seen`.
fn count(seen: &[String], name: &str) -> usize {
    seen.iter().filter(|n| *n == name).count()
}

/// A web-channel event addressed to `client_id`, on `thread_id` when it is
/// not empty.
fn web_event(name: &str, client_id: &str, thread_id: &str) -> WebChannelEvent {
    WebChannelEvent {
        event: name.to_string(),
        client_id: client_id.to_string(),
        thread_id: thread_id.to_string(),
        ..WebChannelEvent::default()
    }
}

/// Runs [`run`] on a large-stack runtime, as the other core-booting suites do.
#[test]
fn every_bridge_reaches_an_authenticated_socket() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(16 * 1024 * 1024)
                .build()
                .unwrap()
                .block_on(run())
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Boots a core, serves the router with Socket.IO on, connects one client and
/// checks each bridge in turn.
async fn run() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        workspace_dir: dir.path().join("workspace"),
        config_path: dir.path().join("config.toml"),
        ..Config::default()
    };
    tinyhumans_boot::boot();
    let _runtime = CoreBuilder::new(HostKind::Library)
        .config(config)
        .token(TokenSource::Fixed(Arc::new(TOKEN.to_string())))
        .services(ServiceSet::none())
        .domains(DomainSet::full())
        .build()
        .await
        .unwrap();
    if BUS.get().is_none() {
        openhuman_core::core::bus::init()
            .await
            .expect("initialize in-process event bus");
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server =
        tokio::spawn(async move { axum::serve(listener, build_core_http_router(true)).await });

    let mut ws = connect(addr).await;

    // Join the thread room straight after connecting, as the app does, then
    // expect both the ack for the join and `ready` with this socket's id.
    ws.send(Message::text(format!(
        "421{}",
        json!(["thread:subscribe", { "thread_id": "t1" }])
    )))
    .await
    .expect("thread:subscribe");
    let mut sid = None;
    let mut joined = false;
    while sid.is_none() || !joined {
        let frame = next_frame(&mut ws).await;
        if let Some(ack) = frame.strip_prefix("431") {
            let ack: Value = serde_json::from_str(ack).expect("ack json");
            assert_eq!(ack, json!([{ "joined": true }]), "{frame}");
            joined = true;
        } else if let Some(("ready", payload)) = event_of(&frame)
            .as_ref()
            .map(|(name, payload)| (name.as_str(), payload))
        {
            sid = Some(
                payload["sid"]
                    .as_str()
                    .expect("ready carries sid")
                    .to_string(),
            );
        }
    }
    let sid = sid.unwrap();

    // Web channel: once the bridge is listening, a frame addressed to a
    // client that is also in the thread room arrives once under each name.
    publish_until(&mut ws, "warmup", || {
        publish_web_channel_event(web_event("warmup", &sid, ""))
    })
    .await;
    publish_web_channel_event(web_event("chat_message", &sid, "t1"));
    publish_web_channel_event(web_event("chat_done", &sid, ""));
    let seen = events_until(&mut ws, "chat_done").await;
    assert_eq!(count(&seen, "chat_message"), 1, "{seen:?}");
    assert_eq!(count(&seen, "chat:message"), 1, "{seen:?}");

    // A frame for another client's turn on the same thread reaches this
    // socket through the thread room alone.
    publish_web_channel_event(web_event("thread_reply", "another-client", "t1"));
    publish_web_channel_event(web_event("thread_done", "another-client", "t1"));
    let seen = events_until(&mut ws, "thread_done").await;
    assert_eq!(count(&seen, "thread_reply"), 1, "{seen:?}");
    assert_eq!(count(&seen, "thread:reply"), 1, "{seen:?}");

    let companion = publish_until(&mut ws, "companion:state_changed", || {
        publish_companion_state_changed(json!({ "state": "listening" }));
    })
    .await;
    assert_eq!(companion["state"], "listening");
    wait_for(&mut ws, "companion_state_changed").await;

    let notification = publish_until(&mut ws, "core_notification", || {
        publish_core_notification(CoreNotificationEvent {
            id: "n1".into(),
            category: CoreNotificationCategory::System,
            title: "title".into(),
            body: "body".into(),
            deep_link: None,
            timestamp_ms: 1,
            actions: None,
            workspace: None,
            workspace_revision: None,
        });
    })
    .await;
    assert_eq!(notification["id"], "n1");
    wait_for(&mut ws, "core:notification").await;

    let bus = BUS.get().expect("event bus");
    let expired = publish_until(&mut ws, "auth:session_expired", || {
        bus.publish(DomainEvent::SessionExpired {
            source: "e2e".into(),
            reason: "test".into(),
        });
    })
    .await;
    assert_eq!(expired["source"], "e2e");
    wait_for(&mut ws, "auth_session_expired").await;

    // The connect seed also sends `workspace_changed`; the revision tells the
    // bridged switch apart from it.
    let switched_dir = dir.path().join("switched");
    let switched = loop {
        let payload = publish_until(&mut ws, "workspace_changed", || {
            bus.publish(DomainEvent::ActiveWorkspaceChanged {
                workspace_dir: switched_dir.clone(),
                revision: 7777,
            });
        })
        .await;
        if payload["revision"] == 7777 {
            break payload;
        }
    };
    assert_eq!(switched["revision"], 7777);
    wait_for(&mut ws, "workspace:changed").await;

    let started = publish_until(&mut ws, "flow:run_started", || {
        bus.publish(DomainEvent::FlowRunStarted {
            flow_id: "f1".into(),
            run_id: "r1".into(),
        });
    })
    .await;
    assert_eq!(started["run_id"], "r1");
    wait_for(&mut ws, "flow_run_started").await;
    bus.publish(DomainEvent::FlowRunProgress {
        run_id: "r1".into(),
        node_id: "n1".into(),
        status: "running".into(),
    });
    assert_eq!(
        wait_for(&mut ws, "flow:run_progress").await["node_id"],
        "n1"
    );
    wait_for(&mut ws, "flow_run_progress").await;
    bus.publish(DomainEvent::FlowRunFinished {
        flow_id: "f1".into(),
        run_id: "r1".into(),
        status: "succeeded".into(),
    });
    assert_eq!(
        wait_for(&mut ws, "flow:run_finished").await["status"],
        "succeeded"
    );
    wait_for(&mut ws, "flow_run_finished").await;
    bus.publish(DomainEvent::FlowChanged {
        flow_id: "f1".into(),
        kind: "updated".into(),
        actor: "user".into(),
    });
    assert_eq!(wait_for(&mut ws, "flow:changed").await["kind"], "updated");
    wait_for(&mut ws, "flow_changed").await;
    bus.publish(DomainEvent::FlowApprovalRequested {
        request_id: "a1".into(),
        flow_id: "f1".into(),
        run_id: "r1".into(),
        tool_name: "shell".into(),
        summary: "run it".into(),
        agent_id: None,
    });
    assert_eq!(
        wait_for(&mut ws, "flow_approval_request").await["request_id"],
        "a1"
    );

    let connected = publish_until(&mut ws, "channel:connection-updated", || {
        bus.publish(DomainEvent::ChannelConnected {
            channel: "discord".into(),
        });
    })
    .await;
    assert_eq!(connected["status"], "connected");
    wait_for(&mut ws, "channel_connection_updated").await;

    server.abort();
}
