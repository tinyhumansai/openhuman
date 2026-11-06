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

/// Regression test for #8458: a sub-agent spawned inside a host-authored
/// turn must reach the thread's socket addressed to the originating client,
/// not vanish into the `system` room. The parent turn's own narration must
/// not cross — only sub-agent events do.
#[tokio::test]
async fn system_turn_subagent_spawn_reaches_the_originating_client() {
    let mut events = crate::web_chat::subscribe_web_channel_events();
    let thread_id = "thread-bgdeliver-spawn";
    let request_id = "bgdeliver-test-spawn";
    let surface = Surface::new(
        &DispatchCtx::ambient(),
        TurnIds {
            client_id: "origin-client".into(),
            thread_id: thread_id.into(),
            request_id: request_id.into(),
        },
        Config::default(),
    )
    .await;
    let (tx, rx) = tokio::sync::mpsc::channel::<AgentProgress>(16);
    let forward = tokio::spawn(forward_system_turn_subagents(surface, rx));

    tx.send(AgentProgress::TextDelta {
        delta: "delivering".into(),
        iteration: 0,
    })
    .await
    .unwrap();
    tx.send(AgentProgress::SubagentSpawned {
        agent_id: "researcher".into(),
        task_id: "sub-bgdeliver".into(),
        mode: "typed".into(),
        dedicated_thread: false,
        prompt_chars: 4,
        prompt: "help".into(),
        worker_thread_id: None,
        display_name: None,
        parent_call_id: None,
    })
    .await
    .unwrap();
    drop(tx);

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
    forward.await.unwrap();
}
