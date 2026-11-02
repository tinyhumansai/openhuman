use super::*;
use crate::core::runtime::DispatchCtx;
use crate::web_chat::ScopedWebEvent;
use std::sync::Arc;

fn ids(request_id: &str) -> TurnIds {
    TurnIds {
        client_id: "surface-client".into(),
        thread_id: format!("surface-thread-{request_id}"),
        request_id: request_id.into(),
    }
}

async fn next_for(
    rx: &mut tokio::sync::broadcast::Receiver<ScopedWebEvent>,
    request_id: &str,
) -> ScopedWebEvent {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
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

fn wire(event: &ScopedWebEvent) -> serde_json::Value {
    serde_json::to_value(event).unwrap()
}

#[tokio::test]
async fn output_events_keep_the_turns_ids_and_one_increasing_seq() {
    let mut rx = crate::web_chat::subscribe_web_channel_events();
    let surface = Surface::new(
        &DispatchCtx::ambient(),
        ids("surface-seq"),
        crate::config::Config::default(),
    )
    .await;
    for text in ["a", "b"] {
        surface
            .output(ChannelOutputEvent::TextDelta {
                text: text.into(),
                iteration: Some(2),
            })
            .await;
    }
    let first = next_for(&mut rx, "surface-seq").await;
    let second = next_for(&mut rx, "surface-seq").await;
    let first_wire = wire(&first);
    assert_eq!(first.event_name(), "text_delta");
    assert_eq!(first.context().client_id, "surface-client");
    assert_eq!(first.context().thread_id, "surface-thread-surface-seq");
    assert_eq!(
        (first_wire["delta"].as_str(), first_wire["round"].as_u64()),
        (Some("a"), Some(2))
    );
    assert_eq!(
        (first.context().seq, second.context().seq),
        (Some(1), Some(2))
    );
    assert!(
        first.context().ts.is_some(),
        "every event carries a wall-clock stamp"
    );
}

#[tokio::test]
async fn an_interim_bubble_carries_the_round_the_canonical_output_lacks() {
    let mut rx = crate::web_chat::subscribe_web_channel_events();
    let surface = Surface::new(
        &DispatchCtx::ambient(),
        ids("surface-interim"),
        crate::config::Config::default(),
    )
    .await;
    surface.interim(3, "Let me check.".into()).await;
    let event = next_for(&mut rx, "surface-interim").await;
    let event_wire = wire(&event);
    assert_eq!(event.event_name(), "chat_interim");
    assert_eq!(event_wire["full_response"], "Let me check.");
    assert_eq!(event_wire["round"], 3);
}

#[tokio::test]
async fn host_authored_events_pass_through_without_a_seq() {
    let mut rx = crate::web_chat::subscribe_web_channel_events();
    let surface = Surface::new(
        &DispatchCtx::ambient(),
        ids("surface-terminal"),
        crate::config::Config::default(),
    )
    .await;
    surface
        .event(bus_web::WebChannelEvent::ChatCancelled {
            context: surface.context(),
            cancel_reason: Some("user_stop".into()),
            superseded_by: None,
        })
        .await;
    let event = next_for(&mut rx, "surface-terminal").await;
    let event_wire = wire(&event);
    assert_eq!(event.event_name(), "chat_cancelled");
    assert_eq!(event_wire["cancel_reason"], "user_stop");
    assert_eq!(event.context().seq, None);
}

#[tokio::test]
async fn direct_and_projected_events_keep_their_historical_wire_normalization() {
    let mut rx = crate::web_chat::subscribe_web_channel_events();
    let direct = Surface::new(
        &DispatchCtx::ambient(),
        ids("surface-direct-null"),
        crate::config::Config::default(),
    )
    .await;
    direct
        .event(bus_web::WebChannelEvent::ApprovalRequest {
            context: direct.context(),
            tool_name: Some(String::new()),
            message: None,
            args: Some(serde_json::Value::Null),
            tool_call_id: None,
            expires_at: None,
            detached: None,
        })
        .await;
    let direct_event = next_for(&mut rx, "surface-direct-null").await;
    let direct_wire = wire(&direct_event);
    assert_eq!(direct_wire["tool_name"], "");
    assert_eq!(
        direct_wire.get("args"),
        Some(&serde_json::Value::Null),
        "host-authored events retain an explicit null args payload"
    );

    let projected = Surface::new(
        &DispatchCtx::ambient(),
        ids("surface-projected-null"),
        crate::config::Config::default(),
    )
    .await;
    projected
        .output(ChannelOutputEvent::ToolCallStarted {
            call_id: "call-null".into(),
            tool_name: String::new(),
            arguments: serde_json::Value::Null,
            iteration: 1,
            display_label: None,
            display_detail: None,
        })
        .await;
    let projected_event = next_for(&mut rx, "surface-projected-null").await;
    let projected_wire = wire(&projected_event);
    assert_eq!(projected_event.event_name(), "tool_call");
    assert!(projected_wire.get("tool_name").is_none());
    assert!(projected_wire.get("args").is_none());
}

#[tokio::test]
async fn module_projected_cancellation_closes_the_turn() {
    let mut rx = crate::web_chat::subscribe_web_channel_events();
    let surface = Surface::new(
        &DispatchCtx::ambient(),
        ids("surface-cancel"),
        crate::config::Config::default(),
    )
    .await;
    surface
        .output(ChannelOutputEvent::Cancellation {
            reason: Some("user_stop".into()),
        })
        .await;

    let event = next_for(&mut rx, "surface-cancel").await;
    assert_eq!(event.event_name(), "chat_cancelled");
    assert_eq!(event.context().seq, Some(1));
    let state = surface.state.lock().await;
    assert!(state.closed);
    assert!(state.handle.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_a_surface_ends_its_module_owned_turn() {
    let surface = Surface::new(
        &DispatchCtx::ambient(),
        ids("surface-drop-cleanup"),
        crate::config::Config::default(),
    )
    .await;
    surface
        .output(ChannelOutputEvent::TextDelta {
            text: "active".into(),
            iteration: Some(1),
        })
        .await;
    let (module, handle, cleanup_done) = {
        let state = surface.state.lock().await;
        (
            state.module.as_ref().unwrap().clone(),
            state.handle.as_ref().unwrap().handle.clone(),
            Arc::clone(&state.cleanup_done),
        )
    };
    let left = surface.clone();
    let right = surface.clone();
    drop(surface);
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let left_barrier = Arc::clone(&barrier);
    let right_barrier = Arc::clone(&barrier);
    let left_drop = std::thread::spawn(move || {
        left_barrier.wait();
        drop(left);
    });
    let right_drop = std::thread::spawn(move || {
        right_barrier.wait();
        drop(right);
    });
    barrier.wait();
    left_drop.join().unwrap();
    right_drop.join().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), cleanup_done.notified())
        .await
        .expect("drop cleanup completes");

    let result: Result<tinychannels_bus::module_web_output::WebOutputEvents, _> = module
        .call_confidential(
            "tinychannels",
            tinychannels_bus::methods::PROJECT_WEB_OUTPUT,
            (ProjectWebOutput {
                handle,
                event: ChannelOutputEvent::TextDelta {
                    text: "late".into(),
                    iteration: Some(1),
                },
            },),
        )
        .await;
    assert!(
        result.is_err(),
        "a dropped turn must reject later projections"
    );
}

#[tokio::test]
async fn the_addressing_context_names_the_turn() {
    let surface = Surface::new(
        &DispatchCtx::ambient(),
        ids("surface-context"),
        crate::config::Config::default(),
    )
    .await;
    let context = surface.context();
    assert_eq!(
        (context.client_id.as_str(), context.request_id.as_str()),
        ("surface-client", "surface-context")
    );
    assert!(context.seq.is_none() && context.ts.is_none());
}
