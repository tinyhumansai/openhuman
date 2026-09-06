use super::*;
use tinyagents_harness::events::AgentEvent;
use tinyagents_harness::ids::ExecutionStatus;

/// The full 05.1 acceptance in miniature: emit events through a sink with a
/// durable journal attached, then reconstruct the timeline + terminal status
/// from the store alone — the "kill the UI mid-turn, reattach, replay" path.
#[tokio::test]
async fn journal_persists_and_replays_run() {
    let tmp = std::env::temp_dir().join(format!("oh-journal-test-{}", uuid::Uuid::new_v4()));
    let stores = open_session_stores(&tmp);
    let run_id = mint_run_id();

    // Attach a journal sink directly (bypassing config resolution) and emit.
    // Seed the sink with the run id so persisted `event_id`s are the
    // restart-stable `{run_id}-evt-{offset}` — mirrors the caller in
    // `run_turn_via_tinyagents_shared`.
    let journal: Arc<dyn HarnessEventJournal> = Arc::new(StoreEventJournal::new(stores.journal));
    let sink = EventSink::with_stream_id(run_id.as_str());
    // Keep a handle to the JournalSink: persistence became asynchronous
    // (background `AppendWorker` drain, tinyagents v1.5 audit remediation),
    // so the test must `flush()` before reading the journal back — exactly
    // the contract the crate documents for read-after-write.
    let journal_sink = Arc::new(JournalSink::new(journal, run_id.clone()));
    let redacting = RedactingSink::new(journal_sink.clone(), vec!["sk-super-secret".into()]);
    sink.subscribe(Arc::new(FanOutSink::new().with(Arc::new(redacting))));

    sink.emit(AgentEvent::ModelStarted {
        call_id: "c1".into(),
        model: "sk-super-secret leaked here".to_string(),
    });
    sink.emit(AgentEvent::ToolStarted {
        call_id: "c1".into(),
        tool_name: "echo".to_string(),
    });
    // Drain the async persistence worker so the durable log has caught up
    // (flush blocks on the drain thread's ack, not on this runtime).
    journal_sink.flush();

    // Reconstruct from the durable store alone.
    let replayed = read_run_events_at(&tmp, run_id.as_str(), 0).await;
    assert_eq!(replayed.len(), 2);
    // Records come back fully ordered with restart-stable ids of the form
    // `{run_id}-evt-{offset}`.
    for (offset, obs) in replayed.iter().enumerate() {
        assert_eq!(obs.offset, offset as u64, "offset should be monotonic");
        assert_eq!(
            obs.event_id.as_str(),
            format!("{}-evt-{offset}", run_id.as_str()),
            "event id should be the stable {{stream_id}}-evt-{{offset}}"
        );
    }
    // The seeded secret was masked before persistence.
    if let AgentEvent::ModelStarted { model, .. } = &replayed[0].event {
        assert!(
            !model.contains("sk-super-secret"),
            "secret should be redacted"
        );
        assert!(model.contains("[REDACTED]"));
    } else {
        panic!("expected ModelStarted first");
    }

    // Late attach at a non-zero offset: a reader that reconnects after the
    // first event reconstructs only the tail (offset >= 1), still ordered and
    // still with stable ids — the mid-run reconnect/backfill path.
    let tail = read_run_events_at(&tmp, run_id.as_str(), 1).await;
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].offset, 1);
    assert_eq!(
        tail[0].event_id.as_str(),
        format!("{}-evt-1", run_id.as_str())
    );
    assert!(matches!(tail[0].event, AgentEvent::ToolStarted { .. }));

    // Status store round-trips a running → completed transition and answers
    // list_active / list_by_root / list_by_thread.
    let status_store = FileStatusStore::new(open_session_stores(&tmp).kv);
    let mut status =
        HarnessRunStatus::new(run_id.clone(), ComponentId::new("mock-model".to_string()))
            .with_thread(ThreadId::new("thread-42"));
    status.mark_running(HarnessPhase::Model);
    status_store.put_status(status.clone()).await.unwrap();
    let active = status_store.list_active().await.unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].status, ExecutionStatus::Running);
    assert_eq!(
        status_store
            .list_by_thread("thread-42")
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(status_store
        .list_by_thread("nope")
        .await
        .unwrap()
        .is_empty());

    status.mark_completed();
    status_store.put_status(status).await.unwrap();
    let by_root = status_store.list_by_root(run_id.as_str()).await.unwrap();
    assert_eq!(by_root.len(), 1);
    assert_eq!(by_root[0].status, ExecutionStatus::Completed);
    assert!(status_store.list_active().await.unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Workspace-parameterized twin of [`read_run_events`] for tests that supply
/// an explicit store root instead of resolving one from config.
async fn read_run_events_at(
    workspace: &std::path::Path,
    run_id: &str,
    from_offset: u64,
) -> Vec<AgentObservation> {
    let stores = open_session_stores(workspace);
    StoreEventJournal::new(stores.journal)
        .read_from(run_id, from_offset)
        .await
        .unwrap()
}

/// Asserts that the durable journal sink and underlying append store handle
/// multi-byte UTF-8 sequences straddling the 4096-byte lookback window boundary
/// without failing validation or dropping observations (#5599).
///
/// `JsonlAppendStore::next_offset` seeks to `len - 4096`. When that byte offset lands
/// mid-character, older implementations using `read_to_string` failed with "stream did
/// not contain valid UTF-8". This test asserts that such boundary straddles are genuinely
/// induced (`assert!(!torn.is_empty())`) and that subsequent appends and late-attach
/// replays succeed contiguously.
#[tokio::test]
async fn journal_sink_handles_multibyte_utf8_spanning_window_boundary() {
    const WINDOW: usize = 4096;
    let mut torn = Vec::new();

    // Walk a padding range to sweep the (len - 4096) lookback boundary across
    // the continuation bytes of a multi-byte UTF-8 sequence.
    for pad in 0..20usize {
        let tmp = std::env::temp_dir().join(format!(
            "oh-journal-utf8-test-{pad}-{}",
            uuid::Uuid::new_v4()
        ));
        let run_id = mint_run_id();

        let stores = open_session_stores(&tmp);
        let journal: Arc<dyn HarnessEventJournal> =
            Arc::new(StoreEventJournal::new(stores.journal));
        let sink = EventSink::with_stream_id(run_id.as_str());
        let journal_sink = Arc::new(JournalSink::new(journal, run_id.clone()));
        sink.subscribe(Arc::new(FanOutSink::new().with(journal_sink.clone())));

        // 10 base events with multi-byte emoji payloads to approach and exceed WINDOW (4096 bytes).
        for i in 0..10 {
            sink.emit(AgentEvent::ModelStarted {
                call_id: format!("call-{i}").into(),
                model: "🦀".repeat(50),
            });
        }

        // An 11th event whose model string is padded byte-by-byte to sweep the window boundary.
        sink.emit(AgentEvent::ModelStarted {
            call_id: "call-10".into(),
            model: "x".repeat(pad),
        });
        journal_sink.flush();

        // Inspect the underlying stream file to verify whether this pad placed the
        // 4096-byte lookback window start inside a multi-byte UTF-8 character.
        let stream_path = tmp
            .join("tinyagents_store")
            .join("journal")
            .join(format!("{}.jsonl", run_id.as_str()));
        let raw = std::fs::read(&stream_path).expect("read stream file");
        if raw.len() > WINDOW {
            let start = raw.len() - WINDOW;
            let text = std::str::from_utf8(&raw).expect("stream file itself is valid UTF-8");
            if !text.is_char_boundary(start) {
                torn.push(pad);
            }
        }

        // Emit a subsequent tool event across the boundary: this forces next_offset
        // to resolve the stream tail and assign the next sequential offset.
        sink.emit(AgentEvent::ToolStarted {
            call_id: "call-11".into(),
            tool_name: "test_tool".to_string(),
        });
        journal_sink.flush();

        // Verify replay: all 12 events must be present and contiguous from offset 0.
        let replayed = read_run_events_at(&tmp, run_id.as_str(), 0).await;
        assert_eq!(
            replayed.len(),
            12,
            "pad {pad}: all observations must be retained"
        );
        for (i, obs) in replayed.iter().enumerate() {
            assert_eq!(
                obs.offset, i as u64,
                "pad {pad}: observation {i} offset must be contiguous"
            );
            assert_eq!(
                obs.event_id.as_str(),
                format!("{}-evt-{i}", run_id.as_str()),
                "pad {pad}: event id should follow stable prefix pattern"
            );
        }

        // Verify model contents on the first and 11th observations match exactly.
        if let AgentEvent::ModelStarted { model, .. } = &replayed[0].event {
            assert_eq!(model, &"🦀".repeat(50));
        } else {
            panic!("pad {pad}: expected ModelStarted at offset 0");
        }
        if let AgentEvent::ModelStarted { model, .. } = &replayed[10].event {
            assert_eq!(model, &"x".repeat(pad));
        } else {
            panic!("pad {pad}: expected ModelStarted at offset 10");
        }
        assert!(matches!(replayed[11].event, AgentEvent::ToolStarted { .. }));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    assert!(
        !torn.is_empty(),
        "no pad put the 4096-byte window start inside a multi-byte character; \
         adjust padding range so the test proves boundary recovery"
    );
}
