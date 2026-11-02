use super::*;
use crate::config::Config;
use crate::web_chat::{ScopedWebEvent, WebChannelEvent};
use std::time::Duration;
use tinyagents_harness::progress::AgentProgress;
use tokio::sync::broadcast::error::TryRecvError;

const MODULE_OUTPUT_TEST_TIMEOUT: Duration = Duration::from_secs(35);

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

/// Attribution for a turn whose stored user payload carries no identity.
mod attribution {
    use super::super::trace::session_profile_user_attribution;

    #[test]
    fn session_profile_attribution_none_when_signed_out() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = crate::config::Config {
            workspace_dir: tmp.path().join("workspace"),
            action_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Default::default()
        };
        assert!(session_profile_user_attribution(&config).is_none());
    }

    #[test]
    fn session_profile_attribution_prefers_email_from_stored_session() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = crate::config::Config {
            workspace_dir: tmp.path().join("workspace"),
            action_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Default::default()
        };
        let service = crate::security::credentials::AuthService::from_config(&config);
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "user_json".to_string(),
            "{\"email\": \"steven@example.test\", \"_id\": \"u-1\"}".to_string(),
        );
        metadata.insert("user_id".to_string(), "u-1".to_string());
        service
            .store_provider_token(
                crate::security::credentials::APP_SESSION_PROVIDER,
                crate::security::credentials::DEFAULT_AUTH_PROFILE_NAME,
                "session-token",
                metadata,
                true,
            )
            .expect("store session profile");
        assert_eq!(
            session_profile_user_attribution(&config).as_deref(),
            Some("steven@example.test"),
            "cold-cache attribution must resolve the on-disk session email"
        );
    }
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
    let mut bus = crate::web_chat::subscribe_web_channel_events();
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

    tx.send(
        tinyagents_harness::progress::AgentProgress::ToolCallCompleted {
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
        },
    )
    .await
    .expect("send progress");

    // The bus is process-global — skip unrelated events from concurrent
    // tests and wait (bounded) for our thread's tool_result.
    let event = tokio::time::timeout(MODULE_OUTPUT_TEST_TIMEOUT, async {
        loop {
            match bus.recv().await {
                Ok(ev)
                    if ev.context().thread_id == "thread-out"
                        && ev.event_name() == "tool_result" =>
                {
                    return ev;
                }
                Ok(_) => continue,
                Err(err) => panic!("bus closed: {err}"),
            }
        }
    })
    .await
    .expect("tool_result within timeout");

    assert!(matches!(
        event.wire_event(),
        crate::web_chat::WebChannelEvent::ToolResult {
            output: Some(output),
            success: Some(true),
            tool_call_id: Some(call_id),
            ..
        } if output == "real payload" && call_id == "call-1"
    ));
}

fn drain_test_config(tmp: &tempfile::TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    }
}

// ── #4270 inference heartbeat ────────────────────────────────────────────

/// Await the next web-channel event published for `thread_id`, skipping
/// events for other threads (the bus is a process-global broadcast) and
/// tolerating broadcast lag. Panics if the channel closes first.
async fn recv_for_thread(
    rx: &mut tokio::sync::broadcast::Receiver<ScopedWebEvent>,
    thread_id: &str,
) -> ScopedWebEvent {
    loop {
        match rx.recv().await {
            Ok(ev) if ev.context().thread_id == thread_id => return ev,
            Ok(_) => continue,
            Err(err) => panic!("web-channel bus closed before event: {err}"),
        }
    }
}

async fn spawn_test_bridge(
    thread_id: &str,
    request_id: &str,
) -> tokio::sync::mpsc::Sender<AgentProgress> {
    spawn_test_bridge_with_interval(
        thread_id,
        request_id,
        Duration::from_secs(INFERENCE_HEARTBEAT_SECS),
    )
    .await
    .0
}

async fn spawn_test_bridge_with_interval(
    thread_id: &str,
    request_id: &str,
    heartbeat_interval: Duration,
) -> (
    tokio::sync::mpsc::Sender<AgentProgress>,
    tokio::task::JoinHandle<()>,
) {
    #[cfg(all(feature = "channels", feature = "modules"))]
    let _module = crate::channels::module_fixture::client().await;
    let (tx, rx) = tokio::sync::mpsc::channel::<AgentProgress>(16);
    let dir = tempfile::tempdir().expect("tempdir");
    let store = std::sync::Arc::new(tinyagents_session::port::InMemoryTurnStates::default())
        as std::sync::Arc<dyn tinyagents_session::port::TurnStates>;
    // Keep the tempdir alive for the bridge task's lifetime by leaking it —
    // a test-only allocation; the OS reclaims it on process exit.
    std::mem::forget(dir);
    let task = spawn_progress_bridge_with_heartbeat_interval(
        rx,
        "client-hb-4270".to_string(),
        thread_id.to_string(),
        request_id.to_string(),
        store,
        ChatRequestMetadata::default(),
        Config::default(),
        crate::core::runtime::DispatchCtx::ambient(),
        heartbeat_interval,
    );
    (tx, task)
}

/// Compaction is announced to the UI as a `context_compacting` web event
/// instead of falling through to the sub-agent handler.
#[tokio::test]
async fn context_compacting_progress_emits_context_compacting_event() {
    let mut events = crate::web_chat::subscribe_web_channel_events();
    let thread_id = "thread-context-compacting";
    let tx = spawn_test_bridge(thread_id, "req-context-compacting").await;
    tx.send(AgentProgress::TurnStarted).await.unwrap();
    tx.send(AgentProgress::ContextCompacting).await.unwrap();
    let mut seen = Vec::new();
    for _ in 0..2 {
        let event = tokio::time::timeout(
            Duration::from_secs(10),
            recv_for_thread(&mut events, thread_id),
        )
        .await
        .expect("bridge should emit its events");
        seen.push(event.event_name().to_owned());
    }
    assert_eq!(seen, ["inference_start", "context_compacting"]);
}

/// Repro-gone guard: once a turn is in flight, the bridge emits a periodic
/// `inference_heartbeat` even though no other progress event has streamed —
/// this is the signal the FE silence timer rearms on to avoid the false
/// "no response after 2 minutes" timeout (#4270).
#[tokio::test]
async fn emits_inference_heartbeat_while_turn_in_flight() {
    let mut events = crate::web_chat::subscribe_web_channel_events();
    let thread_id = "thread-hb-emit-4270";
    let request_id = "req-hb-emit-4270";
    let (tx, task) =
        spawn_test_bridge_with_interval(thread_id, request_id, Duration::from_millis(50)).await;

    // Turn begins — arms the liveness beat.
    tx.send(AgentProgress::TurnStarted).await.unwrap();

    // Keep real time for TinyBus RPC deadlines; the injected short cadence
    // avoids waiting the production 20 seconds.
    let start = tokio::time::timeout(
        Duration::from_secs(10),
        recv_for_thread(&mut events, thread_id),
    )
    .await
    .expect("inference start should reach the web event bus");
    assert_eq!(start.event_name(), "inference_start");

    let beat = tokio::time::timeout(
        Duration::from_secs(10),
        recv_for_thread(&mut events, thread_id),
    )
    .await
    .expect("the active turn should emit its heartbeat");
    assert_eq!(beat.event_name(), "inference_heartbeat");
    assert_eq!(beat.context().thread_id, thread_id);
    assert_eq!(beat.context().request_id, request_id);

    drop(tx);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the bridge should stop after its progress sender closes")
        .expect("the bridge task should complete");
}

/// Lifecycle: once `TurnCompleted` lands the bridge stops beating, so a beat
/// can't race the channel close after the FE has already cleared its timer
/// on `chat_done`/`chat_error`. Exercises the `turn_active = false` arm and
/// the channel-closed `break`.
#[tokio::test]
async fn stops_heartbeat_after_turn_completed() {
    let mut events = crate::web_chat::subscribe_web_channel_events();
    let thread_id = "thread-hb-stop-4270";
    let (tx, task) =
        spawn_test_bridge_with_interval(thread_id, "req-hb-stop-4270", Duration::from_millis(50))
            .await;

    tx.send(AgentProgress::TurnStarted).await.unwrap();
    // Drain through the first heartbeat to prove the turn was beating.
    loop {
        if recv_for_thread(&mut events, thread_id).await.event_name() == "inference_heartbeat" {
            break;
        }
    }

    // Complete the turn, then drop the sender so the bridge loop breaks.
    tx.send(AgentProgress::turn_completed(1)).await.unwrap();
    drop(tx);

    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the bridge should stop after TurnCompleted and channel close")
        .expect("the bridge task should complete");

    loop {
        match events.try_recv() {
            Ok(ev) => assert_ne!(
                (ev.context().thread_id.as_str(), ev.event_name()),
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
#[tokio::test]
async fn no_heartbeat_before_turn_started() {
    let mut events = crate::web_chat::subscribe_web_channel_events();
    let thread_id = "thread-hb-gate-4270";
    let (tx, task) =
        spawn_test_bridge_with_interval(thread_id, "req-hb-gate-4270", Duration::from_millis(25))
            .await;

    // Real time avoids expiring TinyBus' RPC deadline under Tokio's paused
    // clock while still crossing several short heartbeat intervals.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // No event of any kind should have been published for this thread.
    loop {
        match events.try_recv() {
            Ok(ev) => assert_ne!(
                ev.context().thread_id,
                thread_id,
                "unexpected pre-turn event {} for {thread_id}",
                ev.event_name()
            ),
            Err(TryRecvError::Empty | TryRecvError::Closed) => break,
            Err(TryRecvError::Lagged(_)) => continue,
        }
    }

    drop(tx);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the bridge should stop after its progress sender closes")
        .expect("the bridge task should complete");
}

/// Every event the bridge emits carries an additive per-request monotonic
/// `seq` (conversations-timeline-refactor, Phase 4), so the frontend can
/// dedup replayed vs live events by `(request_id, seq)` and order them
/// identically to the persisted snapshot. Drive a short deterministic
/// sequence and assert the emitted seqs are present and strictly increasing.
#[tokio::test]
async fn stamps_monotonic_seq_on_emitted_events() {
    let mut events = crate::web_chat::subscribe_web_channel_events();
    let thread_id = "thread-seq-stamp";
    let request_id = "req-seq-stamp";
    let tx = spawn_test_bridge(thread_id, request_id).await;

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
        assert_eq!(ev.context().request_id, request_id);
        seqs.push(ev.context().seq.expect("every emitted event carries a seq"));
    }
    // The very first emitted event starts the per-request counter at 1.
    assert_eq!(seqs[0], 1, "seq counter starts at 1 for the request");
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "emitted seqs must be strictly increasing, got {seqs:?}"
    );

    drop(tx);
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
    let mut bus = crate::web_chat::subscribe_web_channel_events();
    spawn_progress_bridge(
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

    let interim = tokio::time::timeout(MODULE_OUTPUT_TEST_TIMEOUT, async {
        loop {
            let ev = recv_for_thread(&mut bus, "thread-short-narration").await;
            if ev.event_name() == "chat_interim" {
                return ev;
            }
        }
    })
    .await
    .expect("chat_interim within timeout");
    assert!(matches!(
        interim.wire_event(),
        WebChannelEvent::ChatInterim {
            full_response: Some(text),
            round: Some(1),
            ..
        } if text == "Let me check."
    ));
}

#[tokio::test]
async fn hold_text_stream_suppresses_text_delta_but_keeps_interim() {
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
    let mut bus = crate::web_chat::subscribe_web_channel_events();
    spawn_progress_bridge(
        rx,
        "client-hold".into(),
        "thread-hold".into(),
        "req-hold".into(),
        store,
        ChatRequestMetadata {
            hold_text_stream: true,
            ..Default::default()
        },
        config,
        crate::core::runtime::DispatchCtx::ambient(),
    );

    tx.send(AgentProgress::TextDelta {
        delta: "Let me check the release notes for you first.".into(),
        iteration: 1,
    })
    .await
    .unwrap();
    tx.send(AgentProgress::ToolCallStarted {
        call_id: "tc-hold".into(),
        tool_name: "web_search".into(),
        arguments: serde_json::json!({}),
        iteration: 1,
        display_label: None,
        display_detail: None,
    })
    .await
    .unwrap();

    let events = tokio::time::timeout(MODULE_OUTPUT_TEST_TIMEOUT, async {
        let mut seen = Vec::new();
        loop {
            match bus.recv().await {
                Ok(ev) if ev.context().thread_id == "thread-hold" => {
                    let done = ev.event_name() == "chat_interim";
                    let full_response = match ev.wire_event() {
                        WebChannelEvent::ChatInterim { full_response, .. } => full_response.clone(),
                        _ => None,
                    };
                    seen.push((ev.event_name().to_owned(), full_response));
                    if done {
                        return seen;
                    }
                }
                Ok(_) => continue,
                Err(err) => panic!("bus closed: {err}"),
            }
        }
    })
    .await
    .expect("chat_interim within timeout");

    assert!(!events.iter().any(|(e, _)| e == "text_delta"), "{events:?}");
    // The interim bubble carries the held narration itself.
    let interim = events.iter().find(|(e, _)| e == "chat_interim").unwrap();
    assert_eq!(
        interim.1.as_deref(),
        Some("Let me check the release notes for you first.")
    );
}

mod subagent_tests {
    use super::*;

    #[tokio::test]
    async fn subagent_spawned_forwards_parent_call_id() {
        let mut events = crate::web_chat::subscribe_web_channel_events();
        let thread_id = "thread-c1-spawned";
        let tx = spawn_test_bridge(thread_id, "req-c1-spawned").await;

        tx.send(AgentProgress::SubagentSpawned {
            agent_id: "researcher".into(),
            task_id: "sub-c1".into(),
            mode: "typed".into(),
            dedicated_thread: false,
            prompt_chars: 4,
            prompt: "help".into(),
            worker_thread_id: None,
            display_name: None,
            parent_call_id: Some("call-parent-1".into()),
        })
        .await
        .unwrap();

        let ev = recv_for_thread(&mut events, thread_id).await;
        assert!(matches!(
            ev.wire_event(),
            WebChannelEvent::SubagentSpawned {
                subagent: Some(subagent), ..
            } if subagent.parent_call_id.as_deref() == Some("call-parent-1")
        ));
    }

    /// Terminal sub-agent events (`_completed`/`_failed`/`_awaiting_user`) must
    /// keep carrying the same `parent_call_id` the spawn recorded, even though
    /// those `AgentProgress` variants don't repeat it — the bridge remembers it
    /// per `task_id` (#C1).
    #[tokio::test]
    async fn subagent_completed_carries_parent_call_id_and_capped_output() {
        let mut events = crate::web_chat::subscribe_web_channel_events();
        let thread_id = "thread-c1-completed";
        let tx = spawn_test_bridge(thread_id, "req-c1-completed").await;

        tx.send(AgentProgress::SubagentSpawned {
            agent_id: "researcher".into(),
            task_id: "sub-c1-done".into(),
            mode: "typed".into(),
            dedicated_thread: false,
            prompt_chars: 4,
            prompt: "help".into(),
            worker_thread_id: None,
            display_name: None,
            parent_call_id: Some("call-parent-2".into()),
        })
        .await
        .unwrap();
        let spawned = recv_for_thread(&mut events, thread_id).await;
        assert_eq!(spawned.event_name(), "subagent_spawned");

        tx.send(AgentProgress::SubagentCompleted {
            agent_id: "researcher".into(),
            task_id: "sub-c1-done".into(),
            elapsed_ms: 10,
            iterations: 1,
            output_chars: 5,
            usage: None,
            output: "final answer".into(),
            worktree_path: None,
            changed_files: Vec::new(),
            dirty_status: None,
            stop: None,
        })
        .await
        .unwrap();

        let completed = recv_for_thread(&mut events, thread_id).await;
        assert_eq!(completed.event_name(), "subagent_completed");
        let WebChannelEvent::SubagentCompleted {
            subagent: Some(subagent),
            ..
        } = completed.wire_event()
        else {
            panic!("subagent detail present")
        };
        assert_eq!(subagent.parent_call_id.as_deref(), Some("call-parent-2"));
        assert_eq!(subagent.output.as_deref(), Some("final answer"));
    }

    #[tokio::test]
    async fn subagent_failed_carries_parent_call_id() {
        let mut events = crate::web_chat::subscribe_web_channel_events();
        let thread_id = "thread-c1-failed";
        let tx = spawn_test_bridge(thread_id, "req-c1-failed").await;

        tx.send(AgentProgress::SubagentSpawned {
            agent_id: "researcher".into(),
            task_id: "sub-c1-failed".into(),
            mode: "typed".into(),
            dedicated_thread: false,
            prompt_chars: 4,
            prompt: "help".into(),
            worker_thread_id: None,
            display_name: None,
            parent_call_id: Some("call-parent-3".into()),
        })
        .await
        .unwrap();
        let spawned = recv_for_thread(&mut events, thread_id).await;
        assert_eq!(spawned.event_name(), "subagent_spawned");

        tx.send(AgentProgress::SubagentFailed {
            agent_id: "researcher".into(),
            task_id: "sub-c1-failed".into(),
            error: "boom".into(),
        })
        .await
        .unwrap();

        let failed = recv_for_thread(&mut events, thread_id).await;
        assert_eq!(failed.event_name(), "subagent_failed");
        let WebChannelEvent::SubagentFailed {
            subagent: Some(subagent),
            ..
        } = failed.wire_event()
        else {
            panic!("subagent detail present")
        };
        assert_eq!(subagent.parent_call_id.as_deref(), Some("call-parent-3"));
    }
}

mod tool_progress_tests {
    use super::*;

    /// A running tool's status line reaches the socket as `tool_progress`, tied to
    /// the open call by its id, so the client can show the call is still alive.
    #[tokio::test]
    async fn forwards_tool_progress_for_the_running_call() {
        let mut events = crate::web_chat::subscribe_web_channel_events();
        let thread_id = "thread-tool-progress";
        let tx = spawn_test_bridge(thread_id, "req-tool-progress").await;

        tx.send(AgentProgress::ToolCallProgress {
            call_id: "call-1".into(),
            message: "running 15s · 2.0 KB output".into(),
        })
        .await
        .unwrap();

        let event = recv_for_thread(&mut events, thread_id).await;
        assert_eq!(event.event_name(), "tool_progress");
        assert_eq!(event.context().request_id, "req-tool-progress");
        assert!(matches!(
            event.wire_event(),
            crate::web_chat::WebChannelEvent::ToolProgress {
                tool_call_id: Some(call_id),
                message: Some(message),
                ..
            } if call_id == "call-1" && message == "running 15s · 2.0 KB output"
        ));
        drop(tx);
    }
}
