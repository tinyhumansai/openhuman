use super::interim_narration_text;
use super::session_profile_user_attribution;

#[test]
fn interim_narration_skips_only_empty_text() {
    assert_eq!(interim_narration_text(""), None);
    assert_eq!(interim_narration_text("   \n  "), None);
    // Short narration still flushes: `chat_interim` is what resets the live
    // preview between rounds, so an unflushed "Ok." used to be glued onto the
    // next round's text.
    assert_eq!(interim_narration_text("Ok."), Some("Ok.".to_string()));
    assert_eq!(
        interim_narration_text(" Let me check. "),
        Some("Let me check.".to_string())
    );
}

#[test]
fn interim_narration_surfaces_and_trims_substantial_text() {
    let text = "  Let me check your calendar for conflicts first.  ";
    assert_eq!(
        interim_narration_text(text),
        Some("Let me check your calendar for conflicts first.".to_string())
    );
}

#[tokio::test]
async fn session_profile_attribution_none_when_signed_out() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    };
    assert!(session_profile_user_attribution(&config).await.is_none());
}

#[tokio::test]
async fn session_profile_attribution_prefers_email_from_stored_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    };
    let service = crate::credentials::AuthService::from_config(&config);
    let mut metadata = std::collections::HashMap::new();
    metadata.insert(
        "user_json".to_string(),
        "{\"email\": \"steven@example.test\", \"_id\": \"u-1\"}".to_string(),
    );
    metadata.insert("user_id".to_string(), "u-1".to_string());
    service
        .store_provider_token(
            crate::credentials::APP_SESSION_PROVIDER,
            crate::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "session-token",
            metadata,
            true,
        )
        .await
        .expect("store session profile");
    assert_eq!(
        session_profile_user_attribution(&config).await.as_deref(),
        Some("steven@example.test"),
        "cold-cache attribution must resolve the on-disk session email"
    );
}

use super::*;

#[test]
fn cap_wire_output_passes_through_small_payloads() {
    let s = "small tool result".to_string();
    assert_eq!(cap_wire_output(s.clone()), s);
}

#[test]
fn cap_wire_output_truncates_large_payloads_on_char_boundary() {
    // A multibyte payload past the cap: result stays valid UTF-8, is shorter
    // than the input, and carries the truncation marker.
    let big = "é".repeat(MAX_WIRE_SUBAGENT_OUTPUT); // 2 bytes each → well over cap
    let capped = cap_wire_output(big.clone());
    assert!(capped.len() < big.len());
    assert!(capped.contains("[truncated"));
    // Truncation landed on a char boundary (no replacement char / panic).
    assert!(capped.starts_with('é'));
    // The final payload (content + marker) must honor the wire cap.
    assert!(capped.len() <= MAX_WIRE_SUBAGENT_OUTPUT);
}

/// The `tool_result` wire event must carry the tool's real (capped) output
/// so the UI can render what the tool returned — not the legacy
/// `{"output_chars", "elapsed_ms"}` metadata stub (which broke both the
/// timeline result view and the `propose_workflow` proposal parser).
#[tokio::test]
async fn tool_call_completed_forwards_real_output_on_tool_result() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    };
    let store = std::sync::Arc::new(tinyagents_session::port::InMemoryTurnStates::default())
        as std::sync::Arc<dyn tinyagents_session::port::TurnStates>;
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    let mut bus = super::super::event_bus::subscribe_web_channel_events();
    spawn_progress_bridge(
        rx,
        "client-out".into(),
        "thread-out".into(),
        "req-out".into(),
        store,
        ChatRequestMetadata::default(),
        config,
        crate::core::runtime::DispatchCtx::ambient(),
    );

    tx.send(crate::progress::AgentProgress::ToolCallCompleted {
        call_id: "call-1".into(),
        tool_name: "web_search".into(),
        success: true,
        output_chars: 12,
        output: "real payload".into(),
        arguments: None,
        elapsed_ms: 42,
        iteration: 1,
        failure: None,
        display_label: None,
        display_detail: None,
        structured: None,
    })
    .await
    .expect("send progress");

    // The bus is process-global — skip unrelated events from concurrent
    // tests and wait (bounded) for our thread's tool_result.
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match bus.recv().await {
                Ok(ev) if ev.thread_id == "thread-out" && ev.event == "tool_result" => {
                    return ev;
                }
                Ok(_) => continue,
                Err(err) => panic!("bus closed: {err}"),
            }
        }
    })
    .await
    .expect("tool_result within timeout");

    assert_eq!(event.output.as_deref(), Some("real payload"));
    assert_eq!(event.success, Some(true));
    assert_eq!(event.tool_call_id.as_deref(), Some("call-1"));
}

// ── #4270 inference heartbeat ────────────────────────────────────────────

use crate::config::Config;
use crate::progress::AgentProgress;
use std::time::Duration;
use tokio::sync::broadcast::error::TryRecvError;

/// Await the next web-channel event published for `thread_id`, skipping
/// events for other threads (the bus is a process-global broadcast) and
/// tolerating broadcast lag. Panics if the channel closes first.
async fn recv_for_thread(
    rx: &mut tokio::sync::broadcast::Receiver<WebChannelEvent>,
    thread_id: &str,
) -> WebChannelEvent {
    loop {
        match rx.recv().await {
            Ok(ev) if ev.thread_id == thread_id => return ev,
            Ok(_) => continue,
            Err(err) => panic!("web-channel bus closed before event: {err}"),
        }
    }
}

fn spawn_test_bridge(
    thread_id: &str,
    request_id: &str,
) -> tokio::sync::mpsc::Sender<AgentProgress> {
    let (tx, rx) = tokio::sync::mpsc::channel::<AgentProgress>(16);
    let dir = tempfile::tempdir().expect("tempdir");
    let store = std::sync::Arc::new(tinyagents_session::port::InMemoryTurnStates::default())
        as std::sync::Arc<dyn tinyagents_session::port::TurnStates>;
    // Keep the tempdir alive for the bridge task's lifetime by leaking it —
    // a test-only allocation; the OS reclaims it on process exit.
    std::mem::forget(dir);
    spawn_progress_bridge(
        rx,
        "client-hb-4270".to_string(),
        thread_id.to_string(),
        request_id.to_string(),
        store,
        ChatRequestMetadata::default(),
        Config::default(),
        crate::core::runtime::DispatchCtx::ambient(),
    );
    tx
}

/// Repro-gone guard: once a turn is in flight, the bridge emits a periodic
/// `inference_heartbeat` even though no other progress event has streamed —
/// this is the signal the FE silence timer rearms on to avoid the false
/// "no response after 2 minutes" timeout (#4270).
#[tokio::test(start_paused = true)]
async fn emits_inference_heartbeat_while_turn_in_flight() {
    let mut events = super::super::event_bus::subscribe_web_channel_events();
    let thread_id = "thread-hb-emit-4270";
    let request_id = "req-hb-emit-4270";
    let tx = spawn_test_bridge(thread_id, request_id);

    // Turn begins — arms the liveness beat.
    tx.send(AgentProgress::TurnStarted).await.unwrap();

    // inference_start first, then a heartbeat after the interval elapses
    // (the paused clock auto-advances while the test awaits the bus).
    let start = recv_for_thread(&mut events, thread_id).await;
    assert_eq!(start.event, "inference_start");

    let beat = recv_for_thread(&mut events, thread_id).await;
    assert_eq!(beat.event, "inference_heartbeat");
    assert_eq!(beat.thread_id, thread_id);
    assert_eq!(beat.request_id, request_id);

    drop(tx);
}

/// Lifecycle: once `TurnCompleted` lands the bridge stops beating, so a beat
/// can't race the channel close after the FE has already cleared its timer
/// on `chat_done`/`chat_error`. Exercises the `turn_active = false` arm and
/// the channel-closed `break`.
#[tokio::test(start_paused = true)]
async fn stops_heartbeat_after_turn_completed() {
    let mut events = super::super::event_bus::subscribe_web_channel_events();
    let thread_id = "thread-hb-stop-4270";
    let tx = spawn_test_bridge(thread_id, "req-hb-stop-4270");

    tx.send(AgentProgress::TurnStarted).await.unwrap();
    // Drain through the first heartbeat to prove the turn was beating.
    loop {
        if recv_for_thread(&mut events, thread_id).await.event == "inference_heartbeat" {
            break;
        }
    }

    // Complete the turn, then drop the sender so the bridge loop breaks.
    tx.send(AgentProgress::turn_completed(1)).await.unwrap();
    drop(tx);

    // Let the bridge process TurnCompleted + observe the closed channel.
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    // Advance well past several intervals — no further beats must appear.
    tokio::time::advance(Duration::from_secs(INFERENCE_HEARTBEAT_SECS * 4)).await;
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }

    loop {
        match events.try_recv() {
            Ok(ev) => assert_ne!(
                (ev.thread_id.as_str(), ev.event.as_str()),
                (thread_id, "inference_heartbeat"),
                "heartbeat emitted after TurnCompleted"
            ),
            Err(TryRecvError::Empty | TryRecvError::Closed) => break,
            Err(TryRecvError::Lagged(_)) => continue,
        }
    }
}

/// Gate check: before `TurnStarted` the bridge must NOT beat — otherwise a
/// beat could land before the FE has armed its timer. Exercises the
/// `turn_active == false` branch of the heartbeat tick.
#[tokio::test(start_paused = true)]
async fn no_heartbeat_before_turn_started() {
    let mut events = super::super::event_bus::subscribe_web_channel_events();
    let thread_id = "thread-hb-gate-4270";
    let tx = spawn_test_bridge(thread_id, "req-hb-gate-4270");

    // Advance well past several heartbeat intervals with no TurnStarted.
    tokio::time::advance(Duration::from_secs(INFERENCE_HEARTBEAT_SECS * 4)).await;
    // Let the bridge task run its (no-op) heartbeat ticks.
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }

    // No event of any kind should have been published for this thread.
    loop {
        match events.try_recv() {
            Ok(ev) => assert_ne!(
                ev.thread_id, thread_id,
                "unexpected pre-turn event {} for {thread_id}",
                ev.event
            ),
            Err(TryRecvError::Empty | TryRecvError::Closed) => break,
            Err(TryRecvError::Lagged(_)) => continue,
        }
    }

    drop(tx);
}

/// Every event the bridge emits carries an additive per-request monotonic
/// `seq` (conversations-timeline-refactor, Phase 4), so the frontend can
/// dedup replayed vs live events by `(request_id, seq)` and order them
/// identically to the persisted snapshot. Drive a short deterministic
/// sequence and assert the emitted seqs are present and strictly increasing.
#[tokio::test]
async fn stamps_monotonic_seq_on_emitted_events() {
    let mut events = super::super::event_bus::subscribe_web_channel_events();
    let thread_id = "thread-seq-stamp";
    let request_id = "req-seq-stamp";
    let tx = spawn_test_bridge(thread_id, request_id);

    // Each of these emits exactly one web-channel event: inference_start,
    // tool_call, tool_result (the fast test never trips the 20s heartbeat).
    tx.send(AgentProgress::TurnStarted).await.unwrap();
    tx.send(AgentProgress::ToolCallStarted {
        call_id: "tc-1".into(),
        tool_name: "shell".into(),
        arguments: serde_json::json!({}),
        iteration: 1,
        display_label: None,
        display_detail: None,
    })
    .await
    .unwrap();
    tx.send(AgentProgress::ToolCallCompleted {
        call_id: "tc-1".into(),
        tool_name: "shell".into(),
        success: true,
        output_chars: 0,
        output: String::new(),
        arguments: None,
        elapsed_ms: 5,
        iteration: 1,
        failure: None,
        display_label: None,
        display_detail: None,
        structured: None,
    })
    .await
    .unwrap();

    let mut seqs = Vec::new();
    for _ in 0..3 {
        let ev = recv_for_thread(&mut events, thread_id).await;
        assert_eq!(ev.request_id, request_id);
        seqs.push(ev.seq.expect("every emitted event carries a seq"));
    }
    // The very first emitted event starts the per-request counter at 0.
    assert_eq!(seqs[0], 0, "seq counter starts at 0 for the request");
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "emitted seqs must be strictly increasing, got {seqs:?}"
    );

    drop(tx);
}

// ── Terminal-event ordering: the bridge drains before chat_done ─────────────

fn drain_test_config(tmp: &tempfile::TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    }
}

/// `wait_drained` returns only once the bridge has forwarded every event the
/// parent queued before `TurnCompleted` — the caller publishes `chat_done`
/// right after it, so a tool result must never still be in flight.
#[tokio::test]
async fn wait_drained_returns_after_queued_events_are_forwarded() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = std::sync::Arc::new(tinyagents_session::port::InMemoryTurnStates::default())
        as std::sync::Arc<dyn tinyagents_session::port::TurnStates>;
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    let mut bus = super::super::event_bus::subscribe_web_channel_events();
    let handle = spawn_progress_bridge(
        rx,
        "client-drain".into(),
        "thread-drain".into(),
        "req-drain".into(),
        store,
        ChatRequestMetadata::default(),
        drain_test_config(&tmp),
        crate::core::runtime::DispatchCtx::ambient(),
    );

    tx.send(AgentProgress::ToolCallCompleted {
        call_id: "call-last".into(),
        tool_name: "web_search".into(),
        success: true,
        output_chars: 2,
        output: "ok".into(),
        arguments: None,
        elapsed_ms: 1,
        iteration: 1,
        failure: None,
        display_label: None,
        display_detail: None,
        structured: None,
    })
    .await
    .unwrap();
    tx.send(AgentProgress::turn_completed(1)).await.unwrap();

    // The sender stays alive (as a detached sub-agent's clone would), so the
    // drain must come from `TurnCompleted`, not from the channel closing.
    assert!(handle.wait_drained(Duration::from_secs(5)).await);

    let mut saw_result = false;
    loop {
        match bus.try_recv() {
            Ok(ev) if ev.thread_id == "thread-drain" && ev.event == "tool_result" => {
                saw_result = true;
            }
            Ok(_) | Err(TryRecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
    assert!(
        saw_result,
        "the queued tool_result was published before the drain released"
    );
    drop(tx);
}

/// Without `TurnCompleted` (a failed turn) and with a sender still held, the
/// wait is bounded and reports that it timed out.
#[tokio::test]
async fn wait_drained_is_bounded_when_the_turn_never_completes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = std::sync::Arc::new(tinyagents_session::port::InMemoryTurnStates::default())
        as std::sync::Arc<dyn tinyagents_session::port::TurnStates>;
    let (tx, rx) = tokio::sync::mpsc::channel::<AgentProgress>(16);
    let handle = spawn_progress_bridge(
        rx,
        "client-drain-timeout".into(),
        "thread-drain-timeout".into(),
        "req-drain-timeout".into(),
        store,
        ChatRequestMetadata::default(),
        drain_test_config(&tmp),
        crate::core::runtime::DispatchCtx::ambient(),
    );
    assert!(!handle.wait_drained(Duration::from_millis(50)).await);
    // Closing the channel ends the bridge, which counts as drained.
    drop(tx);
    assert!(handle.wait_drained(Duration::from_secs(5)).await);
}

/// A short narration ("Let me check.") is flushed as `chat_interim` when the
/// round's first tool call starts, so the frontend resets its live preview
/// before the next round streams.
#[tokio::test]
async fn short_narration_is_flushed_on_the_rounds_first_tool_call() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = std::sync::Arc::new(tinyagents_session::port::InMemoryTurnStates::default())
        as std::sync::Arc<dyn tinyagents_session::port::TurnStates>;
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    let mut bus = super::super::event_bus::subscribe_web_channel_events();
    let _handle = spawn_progress_bridge(
        rx,
        "client-short".into(),
        "thread-short-narration".into(),
        "req-short".into(),
        store,
        ChatRequestMetadata::default(),
        drain_test_config(&tmp),
        crate::core::runtime::DispatchCtx::ambient(),
    );
    tx.send(AgentProgress::TextDelta {
        delta: "Let me check.".into(),
        iteration: 1,
    })
    .await
    .unwrap();
    tx.send(AgentProgress::ToolCallStarted {
        call_id: "call-1".into(),
        tool_name: "web_search".into(),
        arguments: serde_json::json!({}),
        iteration: 1,
        display_label: None,
        display_detail: None,
    })
    .await
    .unwrap();

    let interim = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let ev = recv_for_thread(&mut bus, "thread-short-narration").await;
            if ev.event == "chat_interim" {
                return ev;
            }
        }
    })
    .await
    .expect("chat_interim within timeout");
    assert_eq!(interim.full_response.as_deref(), Some("Let me check."));
    assert_eq!(interim.round, Some(1));
}

// ── C1: parent_call_id / capped output forwarding ────────────────────────

#[test]
fn cap_wire_args_passes_through_small_payloads() {
    let args = serde_json::json!({"query": "hello"});
    assert_eq!(cap_wire_args(Some(args.clone())), Some(args));
}

#[test]
fn cap_wire_args_drops_null() {
    assert_eq!(cap_wire_args(Some(serde_json::Value::Null)), None);
    assert_eq!(cap_wire_args(None), None);
}

#[test]
fn cap_wire_args_truncates_oversized_payload_to_a_marker_string() {
    let big = serde_json::json!({ "body": "x".repeat(MAX_WIRE_SUBAGENT_OUTPUT) });
    let capped = cap_wire_args(Some(big)).expect("oversized args still forwarded");
    let rendered = capped.as_str().expect("degrades to a string, not JSON");
    assert!(rendered.len() <= MAX_WIRE_SUBAGENT_OUTPUT);
    assert!(rendered.contains("truncated"));
}

#[path = "progress_bridge_tool_progress_tests.rs"]
mod tool_progress_tests;

#[path = "progress_bridge_subagent_tests.rs"]
mod subagent_tests;

/// A compaction reaches the client as `context_compacting`, for the status
/// line to say what a long pause before the next model call is.
#[tokio::test]
async fn forwards_a_compaction_as_context_compacting() {
    let mut events = super::super::event_bus::subscribe_web_channel_events();
    let thread_id = "thread-context-compacting";
    let request_id = "req-context-compacting";
    let tx = spawn_test_bridge(thread_id, request_id);

    tx.send(AgentProgress::TurnStarted).await.unwrap();
    tx.send(AgentProgress::ContextCompacting).await.unwrap();

    // The bus is process-global and never closes: bound the wait so a missing
    // event fails the test instead of hanging it.
    let compacting = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = recv_for_thread(&mut events, thread_id).await;
            if event.event == "context_compacting" {
                break event;
            }
        }
    })
    .await
    .expect("the bridge forwards ContextCompacting as context_compacting");
    assert_eq!(compacting.request_id, request_id);
    drop(tx);
}
