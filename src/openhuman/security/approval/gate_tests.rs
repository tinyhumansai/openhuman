use super::*;
use tempfile::TempDir;

/// TTL for the tests that assert the *timeout* path — the only ones that
/// need a park to expire while the test is still running.
const EXPIRY_TEST_TTL: Duration = Duration::from_secs(2);

/// Boot-time TTL for the `effective_ttl` fallback tests, which need a
/// window they can name rather than one that has to expire.
///
/// Deliberately a value nothing else in this module uses: those tests read
/// the TTL back out of the gate, so a number shared with the default (or
/// with [`EXPIRY_TEST_TTL`]) would let them pass against the wrong source.
const BOOT_TTL_UNDER_TEST: Duration = Duration::from_secs(7);

/// A gate whose park window is the production [`DEFAULT_APPROVAL_TTL`].
///
/// Most tests here park a call, poll for the row, then decide it; none of
/// them wait for expiry, so the window only has to outlast the poll. It
/// twice did not. #2367 raised it 500ms → 2s after the row expired before
/// `decide` could fire, and 2s then lost the same race under
/// `cargo-llvm-cov`: the poll loops budget 50×10ms, but each
/// `sleep(10ms)` stretches on a contended runner, and once the row is past
/// `expires_at` the `expire_stale_with_now` pass inside [`store::decide`]
/// denies it before that call's own `UPDATE ... WHERE decided_at IS NULL`
/// can match — so `decide` returns `Ok(None)`, the waiter is never woken,
/// and the park resolves as a TTL `Deny`.
///
/// Raising the number a third time would only move the threshold, so the
/// coupling is gone instead: tests that do not exercise expiry cannot
/// reach it, and the five that do ask for [`EXPIRY_TEST_TTL`] explicitly
/// via [`expiry_gate`]. That also removes an undocumented contract
/// between this helper and those distant call sites — two of them still
/// carried a `// TTL = 500ms` comment three raises later.
fn test_gate() -> (ApprovalGate, TempDir) {
    test_gate_with_ttl(DEFAULT_APPROVAL_TTL)
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
    let gate = ApprovalGate::new(config, session, ttl);
    (gate, dir)
}

/// A gate for the tests that let a park expire, plus the env lock they
/// have to hold while it does.
///
/// [`ApprovalGate::effective_ttl`] reads `OPENHUMAN_APPROVAL_TTL_SECS` at
/// park time, not at construction, and this whole suite is a debug build,
/// so the override is live. The `effective_ttl_*` tests set that variable
/// for the length of their assertion. Rust runs tests as threads in one
/// process, so without sharing their lock an expiry test can park under
/// their `42`, and a row that was supposed to die in two seconds outlives
/// the test that is waiting for it instead.
///
/// The guard is returned rather than dropped here: holding it only while
/// the gate is built would leave the window open for the park itself.
/// `test_gate_with_ttl` must not take the lock, because the
/// `effective_ttl_*` tests call it while already holding it.
fn expiry_gate() -> (ApprovalGate, TempDir, std::sync::MutexGuard<'static, ()>) {
    let env = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // The lock keeps a sibling test from setting the override, but not a
    // developer who exported it in their shell. effective_ttl would then
    // replace EXPIRY_TEST_TTL at park time and the wait would be measuring
    // their value, so clear it while the lock is held.
    unsafe { std::env::remove_var("OPENHUMAN_APPROVAL_TTL_SECS") };
    let (gate, dir) = test_gate_with_ttl(EXPIRY_TEST_TTL);
    (gate, dir, env)
}

/// Decide a row that the test has just seen parked, failing on the expiry
/// race rather than through it.
///
/// [`ApprovalGate::decide`] returns `Ok(None)` when the row was already
/// resolved — which, per [`DecideMiss::AlreadyResolved`], includes *lazily
/// expired*. A bare `.unwrap()` there unwraps the `Result`, not the
/// `Option`, so that case passes silently and the test instead fails a few
/// lines later on "the outcome was not `Allow`", naming the wrong event.
fn decide_parked(gate: &ApprovalGate, request_id: &str, decision: ApprovalDecision) {
    assert!(
        gate.decide(request_id, decision).unwrap().is_some(),
        "the parked row {request_id} was already resolved before the decision \
         landed — it expired mid-test rather than being decided"
    );
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
