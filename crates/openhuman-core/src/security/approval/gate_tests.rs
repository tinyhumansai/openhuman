use super::*;
use crate::config::test_env::EnvVarGuard;
use crate::security::approval::engine_stub::StubEngine;
use crate::security::turn_origin;
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

/// The module's park windows, which the scripted engine reproduces (see
/// `engine_stub.rs`): the gate no longer knows them, it asks for no window of
/// its own unless the user configured one.
const MODULE_DEFAULT_WINDOW: Duration = Duration::from_secs(600);
const MODULE_SHORT_WINDOW: Duration = Duration::from_secs(180);

/// A gate that asks for no park window of its own (the module's default
/// applies), over a scripted engine (see `engine_stub.rs`).
///
/// Most tests here park a call, poll for the waiter, then decide it; none of
/// them wait for expiry, so the window only has to outlast the poll. The tests
/// that do exercise expiry ask for [`EXPIRY_TEST_TTL`] explicitly via
/// [`expiry_gate`], so no test depends on how close a shared number sits to
/// the one it needs.
fn test_gate() -> (ApprovalGate, TempDir) {
    build_gate(None)
}

/// Build an approval gate with an explicit park deadline for tests that must
/// coordinate a decision with a concurrently loaded full-suite executor.
fn test_gate_with_ttl(ttl: Duration) -> (ApprovalGate, TempDir) {
    build_gate(Some(ttl))
}

fn build_gate(ttl: Option<Duration>) -> (ApprovalGate, TempDir) {
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
    let stub = StubEngine::new();
    let gate = ApprovalGate::with_engine(config, session, ttl, stub.clone());
    STUBS
        .lock()
        .unwrap()
        .insert(Arc::as_ptr(&gate.parked) as usize, stub);
    (gate, dir)
}

/// The scripted engines by gate, so a test reaches the engine of the gate it built.
static STUBS: std::sync::LazyLock<std::sync::Mutex<HashMap<usize, Arc<StubEngine>>>> =
    std::sync::LazyLock::new(Default::default);

/// The scripted engine behind `gate`.
fn stub_of(gate: &ApprovalGate) -> Arc<StubEngine> {
    STUBS
        .lock()
        .unwrap()
        .get(&(Arc::as_ptr(&gate.parked) as usize))
        .cloned()
        .expect("the gate was built by test_gate")
}

/// Return a request id once the engine delivered the prompt and the waiter has
/// been registered.
fn parked_request_id(gate: &ApprovalGate) -> Option<String> {
    gate.parked.waiters.lock().keys().next().cloned()
}

/// A gate for the tests that let a park expire, plus the env lock they
/// hold until the pending row captures its effective TTL.
///
/// [`ApprovalGate::effective_ttl`] reads `OPENHUMAN_APPROVAL_TTL_SECS` at
/// park time, not at construction, and this whole suite is a debug build,
/// so the override is live. The `effective_ttl_*` tests set that variable
/// for the length of their assertion. Rust runs tests as threads in one
/// process, so without sharing their lock an expiry test can park under
/// their `42`, and a row that was supposed to die in two seconds outlives
/// the test that is waiting for it instead.
///
/// Callers drop the returned guard after observing the pending row. Holding
/// it only while the gate is built would leave the window open for the park's
/// initial `effective_ttl` read, while holding it through expiry serializes
/// these tests unnecessarily.
/// `test_gate_with_ttl` must not take the lock, because the
/// `effective_ttl_*` tests call it while already holding it.
async fn expiry_gate() -> (ApprovalGate, TempDir, EnvVarGuard) {
    // The lock keeps a sibling test from setting the override, but not a
    // developer who exported it in their shell. effective_ttl would then
    // replace EXPIRY_TEST_TTL at park time and the wait would be measuring
    // their value, so clear it while the lock is held.
    let env = EnvVarGuard::locked_unset_async("OPENHUMAN_APPROVAL_TTL_SECS").await;
    let (gate, dir) = test_gate_with_ttl(EXPIRY_TEST_TTL);
    (gate, dir, env)
}

/// Decide a request that the test has just seen parked, failing on the expiry
/// race rather than through it.
///
/// [`ApprovalGate::decide`] returns `Ok(None)` when the request was already
/// resolved — which, per [`DecideMiss::AlreadyResolved`], includes expired. A
/// bare `.unwrap()` there unwraps the `Result`, not the `Option`, so that case
/// passes silently and the test instead fails a few lines later on "the outcome
/// was not `Allow`", naming the wrong event.
fn decide_parked(gate: &ApprovalGate, request_id: &str, decision: ApprovalDecision) {
    assert!(
        gate.decide(request_id, decision).unwrap().is_some(),
        "the parked request {request_id} was already resolved before the decision \
         landed — it expired mid-test rather than being decided"
    );
}

/// A chat context — the gate only parks within a live chat turn now, so
/// tests that exercise parking must run intercept inside this scope.
fn chat_ctx() -> ApprovalChatContext {
    ApprovalChatContext {
        thread_id: "t-test".into(),
        client_id: "c-test".into(),
        request_id: None,
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
        crate::desktop::notifications::types::CoreNotificationEvent,
    >,
    request_id: &str,
) -> crate::desktop::notifications::types::CoreNotificationEvent {
    let expected_id = format!("flow-gate-approval:{request_id}");
    loop {
        match rx.recv().await {
            Ok(event) if event.id == expected_id => return event,
            Ok(_) => continue,
            Err(err) => panic!("the notification bus closed before the approval: {err}"),
        }
    }
}

/// Drain `rx` until an `ApprovalDecided` for `expected_request_id` arrives.
/// Mirrors [`find_flow_approval_requested`]'s filter-not-first-match
/// discipline for the same process-wide-bus reason.
async fn find_approval_decided(
    rx: &mut tinybus::events::EventReceiver<crate::core::events::DomainEvent>,
    expected_request_id: &str,
) -> crate::core::events::DomainEvent {
    loop {
        match rx.recv().await {
            Some(
                ref ev @ crate::core::events::DomainEvent::ApprovalDecided { ref request_id, .. },
            ) if request_id == expected_request_id => return ev.clone(),
            Some(_) => continue,
            None => panic!("the bus closed before the expected event arrived"),
        }
    }
}

#[path = "gate_agent_tests.rs"]
mod agent_tests;
#[path = "gate_core_flow_tests.rs"]
mod core_flow_tests;
#[path = "gate_engine_tests.rs"]
mod engine_tests;
#[path = "gate_forced_tests.rs"]
mod forced_tests;
#[path = "gate_origin_class_tests.rs"]
mod gate_origin_class_tests;
#[path = "gate_origin_intercept_tests.rs"]
mod origin_intercept_tests;
#[path = "gate_subagent_tests.rs"]
mod subagent_tests;
#[path = "gate_tenant_tests.rs"]
mod tenant_tests;
#[path = "gate_triage_tests.rs"]
mod triage_tests;
#[path = "gate_ttl_and_triage_tests.rs"]
mod ttl_and_triage_tests;
#[path = "gate_turn_context_tests.rs"]
mod turn_context_tests;
