//! Sub-agent approval parks: an async-delegated sub-agent's card is routed to
//! the parent chat thread and marked `detached` (so the client keeps it across
//! the parent turn's end), every sub-agent park uses the shorter
//! the module's short window, and an expiry tells the model nobody answered.
//!
//! Production evidence: every failed `media_generate_image` /
//! `media_generate_video` / `skill_registry_install` in `image_agent`,
//! `video_agent` and `skill_setup` ended at exactly 600s with "The approval
//! request expired before anyone responded" — the card was set while the
//! parent turn ran, cleared by its `chat_done`, and never shown again.

use super::*;

/// The origin an async sub-agent inherits: the parent's `WebChat` label.
fn async_subagent_origin(thread: &str) -> AgentTurnOrigin {
    AgentTurnOrigin::WebChat {
        thread_id: thread.into(),
        client_id: "client-sub".into(),
        request_id: Some("parent-turn".into()),
    }
}

async fn requested_event(
    rx: &mut tinybus::events::EventReceiver<crate::core::events::DomainEvent>,
    tool: &str,
) -> crate::core::events::DomainEvent {
    loop {
        match rx.recv().await {
            Some(
                ref ev @ crate::core::events::DomainEvent::ApprovalRequested {
                    ref tool_name, ..
                },
            ) if tool_name == tool => return ev.clone(),
            Some(_) => continue,
            None => panic!("the bus closed before the expected event arrived"),
        }
    }
}

async fn wait_parked(gate: &ApprovalGate) -> PendingApproval {
    for _ in 0..200 {
        if let Some(row) = gate.list_pending().unwrap().into_iter().next() {
            return row;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the call never parked");
}

fn window(row: &PendingApproval, before: chrono::DateTime<chrono::Utc>) -> chrono::Duration {
    row.expires_at.expect("a parked approval sets expires_at") - before
}

#[tokio::test]
async fn an_async_subagent_park_reaches_the_parent_thread_detached_with_a_short_window() {
    let _env = EnvVarGuard::locked_unset_async("OPENHUMAN_APPROVAL_TTL_SECS").await;
    crate::core::bus::init().await.expect("bus init");
    let mut events = crate::core::bus::BUS
        .get()
        .expect("event bus initialized above")
        .receiver();
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let tool = "media_generate_image_async_subagent_test";

    let before = chrono::Utc::now();
    let g = gate.clone();
    // Origin propagated, the chat approval context not — the state
    // `spawn_async_subagent` leaves its detached child in.
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            async_subagent_origin("thread-parent"),
            g.intercept(
                &ApprovalTurnContext::default(),
                tool,
                "generate an image",
                serde_json::json!({}),
            ),
        )
        .await
    });

    let event = requested_event(&mut events, tool).await;
    let crate::core::events::DomainEvent::ApprovalRequested {
        request_id,
        thread_id,
        client_id,
        ..
    } = event
    else {
        unreachable!()
    };
    assert_eq!(
        thread_id.as_deref(),
        Some("thread-parent"),
        "routed to the parent thread"
    );
    assert_eq!(client_id.as_deref(), Some("client-sub"));
    assert!(
        gate.request_is_detached(&request_id),
        "a park that can outlive its turn is marked detached for the surface and replay"
    );

    let row = wait_parked(&gate).await;
    assert!(
        window(&row, before)
            <= chrono::Duration::from_std(MODULE_SHORT_WINDOW).unwrap()
                + chrono::Duration::seconds(5),
        "a sub-agent park uses MODULE_SHORT_WINDOW, got {}",
        window(&row, before)
    );

    decide_parked(&gate, &request_id, ApprovalDecision::ApproveOnce);
    assert!(matches!(handle.await.unwrap(), GateOutcome::Allow));
}

#[tokio::test]
async fn an_inline_chat_park_is_not_detached_and_keeps_the_full_window() {
    let _env = EnvVarGuard::locked_unset_async("OPENHUMAN_APPROVAL_TTL_SECS").await;
    crate::core::bus::init().await.expect("bus init");
    let mut events = crate::core::bus::BUS
        .get()
        .expect("event bus initialized above")
        .receiver();
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let tool = "shell_inline_chat_park_test";

    let before = chrono::Utc::now();
    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            g.intercept(
                &ApprovalTurnContext::chat(chat_ctx()),
                tool,
                "run",
                serde_json::json!({}),
            ),
        )
        .await
    });

    let crate::core::events::DomainEvent::ApprovalRequested { request_id, .. } =
        requested_event(&mut events, tool).await
    else {
        unreachable!()
    };
    assert!(
        !gate.request_is_detached(&request_id),
        "a park inside the chat turn ends with it"
    );
    let row = wait_parked(&gate).await;
    assert!(
        window(&row, before) > chrono::Duration::from_std(MODULE_SHORT_WINDOW).unwrap(),
        "the main chat keeps its full window, got {}",
        window(&row, before)
    );

    decide_parked(&gate, &request_id, ApprovalDecision::ApproveOnce);
    assert!(matches!(handle.await.unwrap(), GateOutcome::Allow));
}

#[tokio::test]
async fn an_inline_subagent_park_uses_the_short_window_without_being_detached() {
    let _env = EnvVarGuard::locked_unset_async("OPENHUMAN_APPROVAL_TTL_SECS").await;
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);

    let before = chrono::Utc::now();
    let g = gate.clone();
    // A blocking sub-agent runs inline under the parent's chat context, one
    // spawn level down.
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            tinyagents_harness::context::with_run_depth(
                tinyagents_harness::context::RunDepth {
                    depth: 1,
                    max_depth: crate::agent::tinyagents::budget::MAX_SPAWN_DEPTH,
                },
                g.intercept(
                    &ApprovalTurnContext::chat(chat_ctx()),
                    "skill_registry_install",
                    "install",
                    serde_json::json!({}),
                ),
            ),
        )
        .await
    });

    let row = wait_parked(&gate).await;
    assert!(!gate.request_is_detached(&row.request_id));
    assert!(
        window(&row, before)
            <= chrono::Duration::from_std(MODULE_SHORT_WINDOW).unwrap()
                + chrono::Duration::seconds(5),
        "got {}",
        window(&row, before)
    );
    decide_parked(&gate, &row.request_id, ApprovalDecision::ApproveOnce);
    assert!(matches!(handle.await.unwrap(), GateOutcome::Allow));
}

#[tokio::test]
async fn an_expired_park_tells_the_model_nobody_answered() {
    let (gate, _dir, env) = expiry_gate().await;
    let gate = Arc::new(gate);
    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            async_subagent_origin("thread-expiry"),
            g.intercept(
                &ApprovalTurnContext::default(),
                "media_generate_video",
                "generate a clip",
                serde_json::json!({}),
            ),
        )
        .await
    });
    let _ = wait_parked(&gate).await;
    drop(env);
    let GateOutcome::Deny { reason } = handle.await.unwrap() else {
        panic!("an unanswered park is denied");
    };
    assert!(is_unanswered_approval_reason(&reason), "{reason}");
    assert!(reason.contains("ask again"), "{reason}");
    // The status classifier still reads it as an expired approval.
    assert_eq!(
        tinytools::status::classify(&reason, false).class,
        tinytools::status::ToolFailureClass::ApprovalExpired,
        "{reason}"
    );
}

/// Two async sub-agents parked on the same parent thread: the thread route
/// holds only the newest, but each park must stay recoverable for replay so a
/// rejoining client can rebuild both cards, and answering one must leave the
/// other addressable.
#[tokio::test]
async fn every_detached_park_on_a_thread_stays_recoverable() {
    let _env = EnvVarGuard::locked_unset_async("OPENHUMAN_APPROVAL_TTL_SECS").await;
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);

    let mut handles = Vec::new();
    for tool in [
        "media_generate_image_multi_a",
        "media_generate_video_multi_b",
    ] {
        let g = gate.clone();
        handles.push(tokio::spawn(async move {
            turn_origin::with_origin(
                async_subagent_origin("thread-multi"),
                g.intercept(
                    &ApprovalTurnContext::default(),
                    tool,
                    "generate media",
                    serde_json::json!({}),
                ),
            )
            .await
        }));
        // Park them one after another so the second overwrites the thread route.
        for _ in 0..200 {
            if gate
                .list_pending()
                .unwrap()
                .iter()
                .any(|row| row.tool_name == tool)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    let parked = gate.parked_requests_for_thread("thread-multi");
    let tools: Vec<_> = parked.iter().map(|row| row.tool_name.as_str()).collect();
    assert_eq!(
        tools,
        vec![
            "media_generate_image_multi_a",
            "media_generate_video_multi_b"
        ],
        "both parks must be replayable, oldest first"
    );
    assert!(gate
        .parked_requests_for_thread("thread-unrelated")
        .is_empty());

    // Answer the newer one; the older one is still parked and still listed.
    decide_parked(&gate, &parked[1].request_id, ApprovalDecision::Deny);
    let remaining = gate.parked_requests_for_thread("thread-multi");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].request_id, parked[0].request_id);

    decide_parked(&gate, &parked[0].request_id, ApprovalDecision::ApproveOnce);
    let first = handles.remove(0).await.unwrap();
    assert!(matches!(first, GateOutcome::Allow));
    assert!(matches!(
        handles.remove(0).await.unwrap(),
        GateOutcome::Deny { .. }
    ));
    assert!(gate.parked_requests_for_thread("thread-multi").is_empty());
}
