use super::*;
use tempfile::TempDir;

fn test_gate() -> (ApprovalGate, TempDir) {
    test_gate_with_ttl(Duration::from_secs(2))
}

/// Build an approval gate with an explicit park deadline for tests that must
/// coordinate a decision with a concurrently loaded full-suite executor.
fn test_gate_with_ttl(ttl: Duration) -> (ApprovalGate, TempDir) {
    let dir = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    // Mirrors the `session-<uuid>` shape minted by
    // `bootstrap_core_runtime` in production so the
    // `debug_assert!` regression guard in `ApprovalGate::new`
    // doesn't trip in tests.
    let session = format!("session-{}", uuid::Uuid::new_v4());
    // 500ms TTL was racing the 50×10ms poll loop on slow CI
    // runners — the row would expire (and get denied by
    // list_pending's lazy-expire) before `decide` could fire,
    // surfacing as "pending row never appeared". 2s gives the
    // most polling tests enough headroom while keeping
    // `timeout_returns_deny` fast (PR #2367 CI flake).
    let gate = ApprovalGate::new(config, session, ttl);
    (gate, dir)
}

/// A chat context — the gate only parks within a live chat turn now, so
/// tests that exercise parking must run intercept inside this scope.
fn chat_ctx() -> ApprovalChatContext {
    ApprovalChatContext {
        thread_id: "t-test".into(),
        client_id: "c-test".into(),
    }
}

/// A matching web-chat origin for the chat context fixture. Tests
/// exercising the parking flow scope BOTH task-locals — production
/// callers in `web_chat` do the same.
fn web_origin() -> AgentTurnOrigin {
    AgentTurnOrigin::WebChat {
        thread_id: "t-test".into(),
        client_id: "c-test".into(),
        request_id: Some("req-test".into()),
    }
}

// ── flow-approval-surface (source_context, flow_tool_trust, surfacing) ──

/// A `Workflow`-origin turn for the flow-correlation tests below.
fn flow_origin(flow_id: &str, require_approval: bool) -> AgentTurnOrigin {
    AgentTurnOrigin::TrustedAutomation {
        job_id: flow_id.to_string(),
        source: TrustedAutomationSource::Workflow { require_approval },
    }
}

/// Drain `rx` until a `FlowApprovalRequested` for `expected_flow_id`
/// arrives. The event bus is process-wide and other tests in this file
/// (and elsewhere) publish on it concurrently — including other
/// `FlowApprovalRequested` events for *different* flow ids — so this must
/// filter by flow id, not just by variant, and tolerate both unrelated
/// events and broadcast lag rather than returning the first match.
async fn find_flow_approval_requested(
    rx: &mut tinybus::events::EventReceiver<crate::core::events::DomainEvent>,
    expected_flow_id: &str,
) -> (String, String, String) {
    loop {
        match rx.recv().await {
            Some(crate::core::events::DomainEvent::FlowApprovalRequested {
                request_id,
                flow_id,
                run_id,
                tool_name,
                ..
            }) if flow_id == expected_flow_id => return (request_id, run_id, tool_name),
            Some(_) => continue,
            None => panic!("the bus closed before the expected event arrived"),
        }
    }
}

/// Drain `rx` until the `flow-gate-approval` notification for
/// `request_id` arrives — the notification bus is process-wide, so
/// unrelated notifications from other concurrently-running tests are
/// tolerated and skipped.
async fn find_flow_gate_notification(
    rx: &mut tokio::sync::broadcast::Receiver<
        crate::openhuman::desktop::notifications::types::CoreNotificationEvent,
    >,
    request_id: &str,
) -> crate::openhuman::desktop::notifications::types::CoreNotificationEvent {
    let expected_id = format!("flow-gate-approval:{request_id}");
    loop {
        match rx.recv().await {
            Ok(event) if event.id == expected_id => return event,
            Ok(_) => continue,
            Err(err) => panic!("the notification bus closed before the approval: {err}"),
        }
    }
}

#[path = "gate_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "gate_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "gate_tests_part_03_tests.rs"]
mod part_03_tests;
