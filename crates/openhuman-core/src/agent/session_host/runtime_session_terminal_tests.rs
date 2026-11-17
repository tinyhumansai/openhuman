use std::sync::Arc;

#[derive(Default)]
struct NativeTerminalRecorder(std::sync::Mutex<Vec<tinyagents_harness::events::AgentEvent>>);

impl tinyagents_harness::events::EventListener for NativeTerminalRecorder {
    fn on_event(&self, record: &tinyagents_harness::events::EventRecord) {
        self.0.lock().unwrap().push(record.event.clone());
    }
}

#[tokio::test]
async fn failed_session_terminal_finalizes_native_run_without_error_text() {
    use tinyagents_harness::events::{AgentEvent, EventSink};
    use tinyagents_harness::ids::RunId;

    let run_id = RunId::new("session-terminal-failure");
    let events = EventSink::with_stream_id(run_id.as_str());
    let recorder = Arc::new(NativeTerminalRecorder::default());
    events.subscribe(recorder.clone());
    let sidecar = std::sync::Mutex::new(
        crate::agent::tinyagents::host::run_context::SessionTurnSidecar {
            deferred_terminal: Some(crate::agent::tinyagents::DeferredTurnTerminal {
                run_id: run_id.clone(),
                events: events.clone(),
                journal: None,
            }),
            ..Default::default()
        },
    );

    crate::agent::session_host::runtime_session::post_commit::finalize_deferred_terminal(
        &sidecar,
        &tinyagents_runtime::SessionTerminal::Failed("private provider detail".into()),
    )
    .await;

    let recorded = recorder.0.lock().unwrap();
    assert!(matches!(
        recorded.as_slice(),
        [
            AgentEvent::RunFailed {
                run_id: failed,
                error,
                outcome: Some(outcome),
            },
            AgentEvent::RunFinalized { run_id: finalized },
        ] if failed == &run_id
            && finalized == &run_id
            && error == "host turn failed after harness completion"
            && outcome.reason == tinyagents_harness::terminal::TerminalReason::Internal
    ));
    assert!(!format!("{recorded:?}").contains("private provider detail"));
    assert!(sidecar.lock().unwrap().deferred_terminal.is_none());
}

#[tokio::test]
async fn cancelled_session_terminal_records_cancelled_native_run() {
    use tinyagents_harness::events::{AgentEvent, EventSink};
    use tinyagents_harness::ids::RunId;

    let run_id = RunId::new("session-terminal-cancelled");
    let events = EventSink::with_stream_id(run_id.as_str());
    let recorder = Arc::new(NativeTerminalRecorder::default());
    events.subscribe(recorder.clone());
    let sidecar = std::sync::Mutex::new(
        crate::agent::tinyagents::host::run_context::SessionTurnSidecar {
            deferred_terminal: Some(crate::agent::tinyagents::DeferredTurnTerminal {
                run_id: run_id.clone(),
                events: events.clone(),
                journal: None,
            }),
            ..Default::default()
        },
    );

    crate::agent::session_host::runtime_session::post_commit::finalize_deferred_terminal(
        &sidecar,
        &tinyagents_runtime::SessionTerminal::Cancelled,
    )
    .await;

    let recorded = recorder.0.lock().unwrap();
    assert!(matches!(
        recorded.as_slice(),
        [
            AgentEvent::RunFailed {
                run_id: failed,
                error,
                outcome: Some(outcome),
            },
            AgentEvent::RunFinalized { run_id: finalized },
        ] if failed == &run_id
            && finalized == &run_id
            && error == "turn cancelled after harness completion"
            && outcome.reason == tinyagents_harness::terminal::TerminalReason::Cancelled
    ));
    assert!(sidecar.lock().unwrap().deferred_terminal.is_none());
}
