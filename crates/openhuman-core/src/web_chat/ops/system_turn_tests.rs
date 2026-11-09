use super::*;
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::web_chat::{ScopedWebEvent, WebChannelEvent};
use std::time::Duration;
use tinyagents_harness::progress::AgentProgress;

#[test]
fn originating_client_id_reads_the_web_client() {
    assert_eq!(
        originating_client_id(r#"{"client_id": "client-1", "thread_id": "t-1"}"#).as_deref(),
        Some("client-1")
    );
}

#[test]
fn originating_client_id_ignores_non_web_contexts() {
    for context in [
        // The system turn's own id: no web client to address to.
        r#"{"client_id": "system", "thread_id": "t-1"}"#,
        // A host built without an event context, and cron/skill/voice ids.
        "standalone",
        "cron:job-1",
        "",
        "not json {",
        r#"{"thread_id": "t-1"}"#,
        r#"{"client_id": ""}"#,
        r#"{"client_id": "  "}"#,
    ] {
        assert_eq!(originating_client_id(context), None, "context: {context}");
    }
}

#[test]
fn forward_client_id_prefers_the_originating_client() {
    assert_eq!(
        forward_client_id(r#"{"client_id": "client-1", "thread_id": "t-1"}"#, "t-1"),
        "client-1"
    );
}

#[test]
fn forward_client_id_falls_back_to_the_thread_not_the_system_room() {
    for context in [
        r#"{"client_id": "system", "thread_id": "t-1"}"#,
        "standalone",
        "cron:job-1",
        "",
        "not json {",
        r#"{"thread_id": "t-1"}"#,
        r#"{"client_id": ""}"#,
    ] {
        assert_eq!(
            forward_client_id(context, "t-1"),
            "t-1",
            "context: {context}"
        );
    }
}

async fn next_for_request(
    rx: &mut tokio::sync::broadcast::Receiver<ScopedWebEvent>,
    request_id: &str,
) -> ScopedWebEvent {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match rx.recv().await {
                Ok(event) if event.context().request_id == request_id => return event,
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(error) => panic!("bus closed: {error}"),
            }
        }
    })
    .await
    .expect("event within timeout")
}

fn spawned(task_id: &str) -> AgentProgress {
    AgentProgress::SubagentSpawned {
        agent_id: "researcher".into(),
        task_id: task_id.into(),
        mode: "typed".into(),
        dedicated_thread: false,
        prompt_chars: 4,
        prompt: "help".into(),
        worker_thread_id: None,
        display_name: None,
        parent_call_id: None,
    }
}

struct Forwarded {
    tx: tokio::sync::mpsc::Sender<AgentProgress>,
    drained_rx: tokio::sync::watch::Receiver<bool>,
    turn_over: Arc<tokio::sync::Notify>,
    forward: tokio::task::JoinHandle<()>,
}

async fn spawn_forwarder(client_id: &str, thread_id: &str, request_id: &str) -> Forwarded {
    let surface = Surface::new(
        &DispatchCtx::ambient(),
        TurnIds {
            client_id: client_id.into(),
            thread_id: thread_id.into(),
            request_id: request_id.into(),
        },
        Config::default(),
    )
    .await;
    let (tx, rx) = tokio::sync::mpsc::channel::<AgentProgress>(16);
    let (drained_tx, drained_rx) = tokio::sync::watch::channel(false);
    let turn_over = Arc::new(tokio::sync::Notify::new());
    let late = LateSubagentSink {
        client_id: client_id.into(),
        thread_id: thread_id.into(),
        request_id: request_id.into(),
        config: Config::default(),
    };
    let forward = tokio::spawn(forward_system_turn_subagents(
        DispatchCtx::ambient(),
        surface,
        rx,
        drained_tx,
        Arc::clone(&turn_over),
        late,
    ));
    Forwarded {
        tx,
        drained_rx,
        turn_over,
        forward,
    }
}

async fn wait_drained(fwd: &mut Forwarded) {
    tokio::time::timeout(
        Duration::from_secs(5),
        fwd.drained_rx.wait_for(|done| *done),
    )
    .await
    .expect("drained within timeout")
    .expect("drain channel open");
}

/// Regression test for #8458: a sub-agent spawned inside a host-authored
/// turn must reach the thread's socket addressed to the originating client,
/// not vanish into the `system` room. The parent turn's own narration must
/// not cross — only sub-agent events do.
#[tokio::test]
async fn system_turn_subagent_spawn_reaches_the_originating_client() {
    let mut events = crate::web_chat::subscribe_web_channel_events();
    let thread_id = "thread-bgdeliver-spawn";
    let request_id = "bgdeliver-test-spawn";
    let fwd = spawn_forwarder("origin-client", thread_id, request_id).await;

    fwd.tx
        .send(AgentProgress::TextDelta {
            delta: "delivering".into(),
            iteration: 0,
        })
        .await
        .unwrap();
    fwd.tx.send(spawned("sub-bgdeliver")).await.unwrap();
    drop(fwd.tx);

    // The first event for this request is the spawn: the narration above it
    // never reaches the socket.
    let event = next_for_request(&mut events, request_id).await;
    assert_eq!(event.event_name(), "subagent_spawned");
    assert_eq!(event.context().client_id, "origin-client");
    assert_eq!(event.context().thread_id, thread_id);
    assert!(matches!(
        event.wire_event(),
        WebChannelEvent::SubagentSpawned {
            subagent: Some(subagent),
            ..
        } if subagent.agent_id.as_deref() == Some("researcher")
            && subagent.task_id.as_deref() == Some("sub-bgdeliver")
    ));
    fwd.forward.await.unwrap();
}

/// With no originating client (cold-booted session), the fallback addresses
/// the thread itself — never the broadcast `system` room every socket joins.
#[tokio::test]
async fn system_turn_fallback_addresses_the_thread_not_the_system_room() {
    let mut events = crate::web_chat::subscribe_web_channel_events();
    let thread_id = "thread-bgdeliver-fallback";
    let request_id = "bgdeliver-test-fallback";
    let fwd = spawn_forwarder(thread_id, thread_id, request_id).await;

    fwd.tx
        .send(spawned("sub-bgdeliver-fallback"))
        .await
        .unwrap();
    drop(fwd.tx);

    let event = next_for_request(&mut events, request_id).await;
    assert_eq!(event.event_name(), "subagent_spawned");
    assert_eq!(event.context().client_id, thread_id);
    assert_eq!(event.context().thread_id, thread_id);
    fwd.forward.await.unwrap();
}

/// A spawn queued while the turn ends must still be published before the
/// surface closes: the drain waits for the terminal event, not for the
/// channel to close — a detached child may hold a sender clone indefinitely.
#[tokio::test]
async fn system_turn_drain_waits_for_the_terminal_event_not_channel_close() {
    let mut events = crate::web_chat::subscribe_web_channel_events();
    let request_id = "bgdeliver-test-drain";
    let mut fwd = spawn_forwarder("origin-client", "thread-bgdeliver-drain", request_id).await;
    // A detached child keeps a sender clone past the parent turn.
    let detached_tx = fwd.tx.clone();
    fwd.tx.send(spawned("sub-bgdeliver-drain")).await.unwrap();
    fwd.tx
        .send(AgentProgress::TurnCompleted {
            iterations: 1,
            stop: None,
        })
        .await
        .unwrap();

    // The drain fires once the terminal event is projected, even though the
    // detached child still holds the channel open.
    wait_drained(&mut fwd).await;

    // ... and the spawn queued ahead of it reached the socket first.
    let event = next_for_request(&mut events, request_id).await;
    assert_eq!(event.event_name(), "subagent_spawned");
    drop(detached_tx);
    drop(fwd.tx);
    fwd.forward.await.unwrap();
}

/// A detached child that completes after its parent turn must still settle
/// its card: the forwarder keeps the receiver as a late sink and publishes
/// terminal sub-agent events through a fresh surface.
#[tokio::test]
async fn system_turn_late_sink_settles_a_detached_child() {
    let mut events = crate::web_chat::subscribe_web_channel_events();
    let request_id = "bgdeliver-test-late";
    let mut fwd = spawn_forwarder("origin-client", "thread-bgdeliver-late", request_id).await;
    let detached_tx = fwd.tx.clone();
    fwd.tx.send(spawned("sub-bgdeliver-late")).await.unwrap();
    fwd.tx
        .send(AgentProgress::TurnCompleted {
            iterations: 1,
            stop: None,
        })
        .await
        .unwrap();

    // The drain fires; the turn surface is now closed.
    wait_drained(&mut fwd).await;

    // The detached child completes after the turn: its completion still lands.
    detached_tx
        .send(AgentProgress::SubagentCompleted {
            agent_id: "researcher".into(),
            task_id: "sub-bgdeliver-late".into(),
            elapsed_ms: 10,
            iterations: 1,
            output_chars: 4,
            usage: None,
            output: "done".into(),
            worktree_path: None,
            changed_files: vec![],
            dirty_status: None,
            stop: None,
        })
        .await
        .unwrap();

    // The spawn (full mode) first, then the late completion.
    let first = next_for_request(&mut events, request_id).await;
    assert_eq!(first.event_name(), "subagent_spawned");
    let event = next_for_request(&mut events, request_id).await;
    assert_eq!(event.event_name(), "subagent_completed");
    assert_eq!(event.context().client_id, "origin-client");
    assert_eq!(event.context().thread_id, "thread-bgdeliver-late");
    drop(detached_tx);
    drop(fwd.tx);
    fwd.forward.await.unwrap();
}

/// Regression test for the CodeRabbit finding on #8565: spawns queued
/// when the turn ends must reach the socket in full mode, not be dropped
/// by the late sink. The spawns and the turn-over signal are issued
/// back-to-back with no await between them, so the pump may not have
/// moved the spawns yet when the turn ends; the FIFO turn-over marker
/// guarantees both are projected, in order, before the switch either way.
#[tokio::test]
async fn system_turn_spawns_queued_at_turn_over_reach_the_socket() {
    let mut events = crate::web_chat::subscribe_web_channel_events();
    let request_id = "bgdeliver-test-turn-over-spawns";
    let mut fwd = spawn_forwarder(
        "origin-client",
        "thread-bgdeliver-turn-over-spawns",
        request_id,
    )
    .await;
    let detached_tx = fwd.tx.clone();
    // Two spawns, then the turn ends immediately: neither may be lost to
    // the switch, and both arrive in order.
    fwd.tx
        .send(spawned("sub-bgdeliver-turn-over-a"))
        .await
        .unwrap();
    fwd.tx
        .send(spawned("sub-bgdeliver-turn-over-b"))
        .await
        .unwrap();
    fwd.turn_over.notify_one();

    for task_id in ["sub-bgdeliver-turn-over-a", "sub-bgdeliver-turn-over-b"] {
        let event = next_for_request(&mut events, request_id).await;
        assert_eq!(event.event_name(), "subagent_spawned");
        assert!(matches!(
            event.wire_event(),
            WebChannelEvent::SubagentSpawned {
                subagent: Some(subagent),
                ..
            } if subagent.task_id.as_deref() == Some(task_id)
        ));
    }

    // The drain fired once the marker was reached; a detached child
    // completing after the switch still settles through the late sink.
    wait_drained(&mut fwd).await;
    detached_tx
        .send(AgentProgress::SubagentCompleted {
            agent_id: "researcher".into(),
            task_id: "sub-bgdeliver-turn-over-a".into(),
            elapsed_ms: 10,
            iterations: 1,
            output_chars: 4,
            usage: None,
            output: "done".into(),
            worktree_path: None,
            changed_files: vec![],
            dirty_status: None,
            stop: None,
        })
        .await
        .unwrap();
    let event = next_for_request(&mut events, request_id).await;
    assert_eq!(event.event_name(), "subagent_completed");
    drop(detached_tx);
    drop(fwd.tx);
    fwd.forward.await.unwrap();
}

/// A turn that ends without a terminal event (error path) still switches
/// the forwarder to its late sink once the caller signals it, instead of
/// leaving the turn surface open with a detached child unheard.
#[tokio::test]
async fn system_turn_turn_over_switches_to_the_late_sink() {
    let mut events = crate::web_chat::subscribe_web_channel_events();
    let request_id = "bgdeliver-test-turn-over";
    let fwd = spawn_forwarder("origin-client", "thread-bgdeliver-turn-over", request_id).await;
    let detached_tx = fwd.tx.clone();
    fwd.tx
        .send(spawned("sub-bgdeliver-turn-over"))
        .await
        .unwrap();
    // No TurnCompleted: the turn failed. Signal it over.
    fwd.turn_over.notify_one();

    // The queued spawn was still published in full mode before the switch.
    let first = next_for_request(&mut events, request_id).await;
    assert_eq!(first.event_name(), "subagent_spawned");

    // And a detached child completing after the switch still settles.
    detached_tx
        .send(AgentProgress::SubagentCompleted {
            agent_id: "researcher".into(),
            task_id: "sub-bgdeliver-turn-over".into(),
            elapsed_ms: 10,
            iterations: 1,
            output_chars: 4,
            usage: None,
            output: "done".into(),
            worktree_path: None,
            changed_files: vec![],
            dirty_status: None,
            stop: None,
        })
        .await
        .unwrap();
    let event = next_for_request(&mut events, request_id).await;
    assert_eq!(event.event_name(), "subagent_completed");
    drop(detached_tx);
    drop(fwd.tx);
    fwd.forward.await.unwrap();
}
