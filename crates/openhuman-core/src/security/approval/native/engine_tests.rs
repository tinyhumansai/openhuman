//! The native engine against the real TinySecurity module.
//!
//! These tests load the actual native library behind an in-memory broker with
//! the host's own approval initialization and callbacks, and drive the engine
//! the gate drives. They need a released archive (`OPENHUMAN_TEST_SECURITY_RELEASE_HOST`)
//! or an explicitly selected local fixture (`OPENHUMAN_TEST_SECURITY_MODULE`), so
//! they are ignored by default.

use super::*;
use crate::modules::security::native_tests::{native_approval_fixture, NativeFixture};
use crate::security::approval::engine::{Delivery, Posture, PromptSurface};
use crate::security::approval::types::ApprovalDecision;
use std::sync::{Arc, Mutex};
use tinysecurity_bus::{names, ApprovalLifecycle2, ApprovalRow2, DecideRequest2};

#[path = "engine_tool_trust_tests.rs"]
mod tool_trust_tests;

/// A prompt surface that records what the engine asked it to show.
#[derive(Default)]
pub(super) struct Shown {
    deliveries: Mutex<Vec<Delivery>>,
}

#[async_trait]
impl PromptSurface for Shown {
    async fn deliver(&self, delivery: Delivery) -> Result<(), String> {
        self.deliveries.lock().unwrap().push(delivery);
        Ok(())
    }
}

pub(super) struct World {
    fixture: NativeFixture,
    _dir: tempfile::TempDir,
    engine: NativeEngine,
    config: Config,
}

pub(super) async fn world() -> World {
    let fixture = native_approval_fixture().await;
    let dir = tempfile::tempdir_in(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp"),
    )
    .unwrap_or_else(|_| tempfile::tempdir().unwrap());
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let engine = NativeEngine::on(config.clone(), fixture.proxy.clone());
    World {
        fixture,
        _dir: dir,
        engine,
        config,
    }
}

pub(super) fn input(tool: &str, surface: &Arc<Shown>) -> CheckInput {
    CheckInput {
        tenant: crate::core::runtime::Tenant::default(),
        variant: Variant::Standard,
        posture: Posture {
            require_approval: true,
            ..Posture::default()
        },
        legacy_tool_grants: Vec::new(),
        call_id: format!("{:08x}", rand_id()),
        tool: tool.to_string(),
        args: serde_json::json!({"to": "someone", "token": "not-a-real-token-value"}),
        summary: "send a message".into(),
        thread: Some("thread-1".into()),
        flow: None,
        origin: CallOrigin::Operator,
        ttl: Some(Duration::from_secs(600)),
        prompt: Arc::clone(surface) as Arc<dyn PromptSurface>,
    }
}

fn rand_id() -> u32 {
    let bytes = uuid::Uuid::new_v4().into_bytes();
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn pending(checked: Checked) -> (String, u64) {
    match checked {
        Checked::Pending {
            request_id,
            expires_at_ms,
        } => (request_id, expires_at_ms),
        other => panic!("expected a pending request, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_person_approves_reserves_and_the_outcome_is_recorded() {
    let world = world().await;
    let shown = Arc::new(Shown::default());
    let (request_id, expires_at_ms) = pending(
        world
            .engine
            .check(input("gmail_send", &shown))
            .await
            .expect("the module evaluates a host tool"),
    );

    // The module made the request durable and the prompt reached the surface,
    // with what the module stored (redacted), not what the host sent.
    let deliveries = shown.deliveries.lock().unwrap().clone();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].request_id, request_id);
    assert_eq!(deliveries[0].tool_name, "gmail_send");
    assert!(
        !deliveries[0]
            .args_redacted
            .to_string()
            .contains("not-a-real-token-value"),
        "the stored arguments are redacted"
    );
    assert!(expires_at_ms > deliveries[0].created_at_ms);
    assert!(
        expires_at_ms - deliveries[0].created_at_ms <= 600_000,
        "the interactive window never exceeds ten minutes"
    );

    assert_eq!(
        world
            .engine
            .decide(&request_id, Choice::Once, "approve_once")
            .await
            .unwrap(),
        Decided::Approved
    );
    world.engine.reserve(&request_id).await.unwrap();
    assert!(
        world.engine.reserve(&request_id).await.is_err(),
        "an approval is reserved exactly once"
    );
    world
        .engine
        .record_execution(&request_id, ExecutionOutcome::Success)
        .await
        .unwrap();

    let history = world.engine.history(10).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].request_id, request_id);
    assert_eq!(history[0].decision, ApprovalDecision::ApproveOnce);
    assert!(history[0].decided_by.is_none(), "a person decided");
    drop(world.fixture);
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_denial_leaves_nothing_to_reserve() {
    let world = world().await;
    let shown = Arc::new(Shown::default());
    let (request_id, _) = pending(
        world
            .engine
            .check(input("gmail_send", &shown))
            .await
            .unwrap(),
    );
    assert_eq!(
        world
            .engine
            .decide(&request_id, Choice::Deny, "deny")
            .await
            .unwrap(),
        Decided::Denied
    );
    assert!(world.engine.reserve(&request_id).await.is_err());
    let history = world.engine.history(10).await.unwrap();
    assert_eq!(history[0].decision, ApprovalDecision::Deny);
    // Deciding again finds nothing pending.
    assert_eq!(
        world
            .engine
            .decide(&request_id, Choice::Once, "approve_once")
            .await,
        Err(EngineError::NotPending)
    );
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn the_users_tool_grant_approves_without_a_prompt_and_is_audited() {
    let world = world().await;
    let shown = Arc::new(Shown::default());
    let mut granted = input("gmail_send", &shown);
    granted.legacy_tool_grants = vec!["gmail_send".into()];
    let checked = world.engine.check(granted).await.unwrap();
    let Checked::Approved { request_id } = checked else {
        panic!("a granted tool is approved without a human: {checked:?}");
    };
    assert!(
        shown.deliveries.lock().unwrap().is_empty(),
        "nobody was asked"
    );
    world.engine.reserve(&request_id).await.unwrap();
    world
        .engine
        .record_execution(&request_id, ExecutionOutcome::Failure)
        .await
        .unwrap();
    let history = world.engine.history(10).await.unwrap();
    assert_eq!(history[0].decided_by.as_deref(), Some("auto:tool-grant"));

    // Another tool is not covered by the grant.
    let mut other = input("slack_post", &shown);
    other.legacy_tool_grants = vec!["gmail_send".into()];
    assert!(matches!(
        world.engine.check(other).await.unwrap(),
        Checked::Pending { .. }
    ));
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn an_imported_tool_grant_survives_an_empty_transient_grant_list() {
    let world = world().await;
    let shown = Arc::new(Shown::default());
    let mut granted = input("gmail_send", &shown);
    granted.legacy_tool_grants = vec!["gmail_send".into()];
    assert!(matches!(
        world.engine.check(granted).await.unwrap(),
        Checked::Approved { .. }
    ));
    // The source import is durable; a later caller need not carry the legacy
    // posture grant for the module to retain that imported trust.
    assert!(matches!(
        world
            .engine
            .check(input("gmail_send", &shown))
            .await
            .unwrap(),
        Checked::Approved { .. }
    ));
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_flow_grant_applies_to_that_flow_only() {
    let world = world().await;
    let shown = Arc::new(Shown::default());
    world
        .engine
        .grant_flow_trust(
            &crate::core::runtime::Tenant::default(),
            "flow-1",
            "http_request",
        )
        .await
        .expect("the module keeps the flow trust");
    let mut in_flow = input("http_request", &shown);
    in_flow.variant = Variant::FlowStrict;
    in_flow.flow = Some("flow-1".into());
    // The global tool list never approves a strict flow, only its own trust.
    in_flow.legacy_tool_grants = vec!["slack_post".into()];
    assert!(matches!(
        world.engine.check(in_flow).await.unwrap(),
        Checked::Approved { .. }
    ));
    let mut other_flow = input("http_request", &shown);
    other_flow.variant = Variant::FlowStrict;
    other_flow.flow = Some("flow-2".into());
    assert!(matches!(
        world.engine.check(other_flow).await.unwrap(),
        Checked::Pending { .. }
    ));
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_durable_operator_tool_grant_is_reused_by_the_module() {
    let world = world().await;
    let shown = Arc::new(Shown::default());
    world
        .engine
        .grant_tool_trust(
            &crate::core::runtime::Tenant::default(),
            Variant::Standard,
            "gmail_send",
        )
        .await
        .expect("TinySecurity persists the exact operator grant");
    assert!(matches!(
        world
            .engine
            .check(input("gmail_send", &shown))
            .await
            .unwrap(),
        Checked::Approved { .. }
    ));
    assert!(shown.deliveries.lock().unwrap().is_empty());
    assert!(matches!(
        world
            .engine
            .check(input("slack_post", &shown))
            .await
            .unwrap(),
        Checked::Pending { .. }
    ));
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn legacy_auto_approve_is_imported_as_durable_tool_trust() {
    let world = world().await;
    let shown = Arc::new(Shown::default());
    let mut legacy = input("gmail_send", &shown);
    legacy.legacy_tool_grants = vec!["gmail_send".into()];

    assert!(matches!(
        world.engine.check(legacy).await.unwrap(),
        Checked::Approved { .. }
    ));

    // Simulate the next process: legacy config still carries the item, but no
    // transient scope registration grants it. Only the import receipt can keep
    // it active in the module.
    let restarted = NativeEngine::on(world.config.clone(), world.fixture.proxy.clone());
    let migrated = input("gmail_send", &shown);
    assert!(
        matches!(
            restarted.check(migrated).await.unwrap(),
            Checked::Approved { .. }
        ),
        "the imported grant must survive without being re-granted by posture"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn auto_approve_all_is_the_posture_and_a_forced_call_ignores_it() {
    let world = world().await;
    let shown = Arc::new(Shown::default());
    let mut all = input("gmail_send", &shown);
    all.posture.auto_approve_all = true;
    all.legacy_tool_grants = vec!["gmail_send".into()];
    let checked = world.engine.check(all).await.unwrap();
    assert!(matches!(checked, Checked::Approved { .. }));
    let history = {
        let Checked::Approved { request_id } = checked else {
            unreachable!()
        };
        world.engine.reserve(&request_id).await.unwrap();
        world.engine.history(10).await.unwrap()
    };
    assert_eq!(history[0].decided_by.as_deref(), Some("auto:host-policy"));

    // A forced call asks for a person whatever the user configured.
    let mut forced = input("gmail_send", &shown);
    forced.variant = Variant::Forced;
    forced.posture = Posture {
        require_approval: true,
        ..Posture::default()
    };
    assert!(matches!(
        world.engine.check(forced).await.unwrap(),
        Checked::Pending { .. }
    ));
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_request_nobody_answers_expires_and_is_recorded_as_denied() {
    let world = world().await;
    let shown = Arc::new(Shown::default());
    let mut short = input("gmail_send", &shown);
    short.ttl = Some(Duration::from_millis(1200));
    let (request_id, expires_at_ms) = pending(world.engine.check(short).await.unwrap());
    assert!(
        world.engine.expire(&request_id).await.is_err(),
        "a live request is not expired early"
    );
    let wait = expires_at_ms.saturating_sub(now_ms()) + 50;
    tokio::time::sleep(Duration::from_millis(wait)).await;
    assert!(world.engine.expire(&request_id).await.unwrap());
    let history = world.engine.history(10).await.unwrap();
    assert_eq!(history[0].decision, ApprovalDecision::Deny);
    assert!(
        world.engine.reserve(&request_id).await.is_err(),
        "an expired request cannot be reserved"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_cancelled_request_cannot_be_decided() {
    let world = world().await;
    let shown = Arc::new(Shown::default());
    let (request_id, _) = pending(
        world
            .engine
            .check(input("gmail_send", &shown))
            .await
            .unwrap(),
    );
    world.engine.cancel(&request_id).await.unwrap();
    assert_eq!(
        world
            .engine
            .decide(&request_id, Choice::Once, "approve_once")
            .await,
        Err(EngineError::NotPending)
    );
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_reserved_tool_name_is_refused_as_a_host_tool() {
    let world = world().await;
    let shown = Arc::new(Shown::default());
    // The module's own effect classes and human-only tools are not host tools.
    for tool in ["local_diagnostic", "native_secret_grant", "device_crypto"] {
        assert!(
            matches!(
                world.engine.check(input(tool, &shown)).await,
                Err(EngineError::Unavailable(_))
            ),
            "{tool}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn concurrent_calls_of_one_subject_all_get_their_own_request() {
    let world = world().await;
    let engine = Arc::new(NativeEngine::on(
        world.config.clone(),
        world.fixture.proxy.clone(),
    ));
    let mut tasks = Vec::new();
    for index in 0..8 {
        let engine = Arc::clone(&engine);
        tasks.push(tokio::spawn(async move {
            let shown = Arc::new(Shown::default());
            let mut call = input("gmail_send", &shown);
            call.args = serde_json::json!({ "n": index });
            pending(engine.check(call).await.expect("concurrent check")).0
        }));
    }
    let mut ids = Vec::new();
    for task in tasks {
        ids.push(task.await.unwrap());
    }
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 8, "each call got its own durable request");
    // The audit stream stayed contiguous: one event per request.
    let scope = {
        let scopes = super::super::state::host().scopes.lock().await;
        scopes
            .by_subject
            .values()
            .find(|entry| entry.subject.workspace == world.config.workspace_dir.to_string_lossy())
            .cloned()
            .expect("the subject is registered")
    };
    assert_eq!(scope.rows.tail(&scope.stream_id).await.unwrap(), 8);
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn two_workspaces_are_two_subjects_with_their_own_rows() {
    let world = world().await;
    let other_dir = tempfile::tempdir().unwrap();
    let other_config = Config {
        workspace_dir: other_dir.path().to_path_buf(),
        ..Config::default()
    };
    let other = NativeEngine::on(other_config, world.fixture.proxy.clone());
    let shown = Arc::new(Shown::default());
    let (mine, _) = pending(
        world
            .engine
            .check(input("gmail_send", &shown))
            .await
            .unwrap(),
    );
    let (theirs, _) = pending(other.check(input("gmail_send", &shown)).await.unwrap());
    assert_ne!(mine, theirs);
    // Neither workspace's history shows the other's request.
    world
        .engine
        .decide(&mine, Choice::Once, "approve_once")
        .await
        .unwrap();
    assert_eq!(world.engine.history(10).await.unwrap().len(), 1);
    assert!(other.history(10).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_ticket_the_host_never_issued_decides_nothing() {
    let world = world().await;
    let shown = Arc::new(Shown::default());
    let (request_id, _) = pending(
        world
            .engine
            .check(input("gmail_send", &shown))
            .await
            .unwrap(),
    );
    let entry = super::super::state::host()
        .request(&request_id)
        .expect("the delivered request is remembered");

    // What a model could try: the module's own `Decide2` with a ticket it made up.
    let forged = {
        let _guard = entry.scope.stream.lock().await;
        let generation = NativeEngine::generation(&world.fixture.proxy)
            .await
            .unwrap();
        let audit = NativeEngine::correlation(&entry.scope, generation)
            .await
            .unwrap();
        world
            .fixture
            .proxy
            .call_confidential::<ApprovalRow2>(
                names::methods::DECIDE2,
                (DecideRequest2 {
                    scope_id: entry.scope.scope_id.clone(),
                    request_id: request_id.clone(),
                    decision_ticket: "model-claimed-human".into(),
                    audit,
                },),
            )
            .await
    };
    assert!(
        forged.is_err(),
        "the host refused to verify a forged ticket"
    );
    let stored = entry.scope.rows.row(&request_id).await.unwrap().unwrap();
    assert_eq!(stored.row.state, ApprovalLifecycle2::Pending);
    assert!(world.engine.reserve(&request_id).await.is_err());

    // The real decision still works afterwards.
    assert_eq!(
        world
            .engine
            .decide(&request_id, Choice::Once, "approve_once")
            .await
            .unwrap(),
        Decided::Approved
    );
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn only_the_module_may_call_the_hosts_approval_callbacks() {
    let world = world().await;
    let stranger = tinybus::Connection::connect(world.fixture.transport.connect().await.unwrap())
        .await
        .unwrap();
    let host_peer = world.fixture.client.unique_name().expect("host peer");
    for (interface, member) in [
        ("ai.tinyhumans.tinysecurity.ApprovalStore2", "Load"),
        ("ai.tinyhumans.tinysecurity.AuditSink2", "Append"),
        ("ai.tinyhumans.tinysecurity.ApprovalPrompt2", "VerifyHuman"),
    ] {
        let proxy = stranger
            .proxy(
                host_peer.as_str(),
                tinysecurity_bus::callback_names2::OBJECT_PATH,
                interface,
            )
            .unwrap();
        let answered = proxy
            .call::<serde_json::Value>(member, (serde_json::json!({}),))
            .await;
        assert!(
            answered.is_err(),
            "{interface}.{member} answered a stranger"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn the_legacy_history_and_flow_trust_are_carried_into_the_module_once() {
    use crate::security::approval::native::history_import;
    use crate::security::approval::store_import::with_connection;
    use crate::storage::rusqlite::params;

    let world = world().await;
    with_connection(&world.config, |conn| {
        let now = chrono::Utc::now().to_rfc3339();
        for (id, decision) in [("r1", "approve_once"), ("r2", "deny")] {
            conn.execute(
                "INSERT INTO pending_approvals
                    (request_id, tool_name, action_summary, args_redacted, session_id,
                     created_at, expires_at, decided_at, decision)
                 VALUES (?1, 'composio', 'a summary', '{}', 'sess', ?2, ?2, ?2, ?3)",
                params![id, now, decision],
            )?;
        }
        conn.execute(
            "INSERT INTO flow_tool_trust (flow_id, tool_name, created_at) VALUES ('flow-1', 'shell', ?1)",
            params![now],
        )?;
        Ok(())
    })
    .unwrap();

    let report = history_import::run(&world.engine, &world.fixture.proxy)
        .await
        .expect("the module takes the import")
        .expect("there was a legacy file");
    assert_eq!((report.history, report.trust, report.skipped), (2, 1, 0));

    // The flow trust is the module's now, and the history reads as what it was.
    let tenant = crate::core::runtime::Tenant::default();
    assert_eq!(
        world.engine.flow_trust(&tenant, "flow-1").await.unwrap(),
        ["shell"]
    );
    let history = world.engine.history(50).await.unwrap();
    let decided = |decision: ApprovalDecision| {
        history
            .iter()
            .filter(|entry| entry.decision == decision)
            .count()
    };
    assert_eq!(decided(ApprovalDecision::ApproveOnce), 1);
    assert_eq!(decided(ApprovalDecision::Deny), 1);
    assert_eq!(decided(ApprovalDecision::ApproveAlwaysForFlow), 1);
    let initial_history_count = history.len();

    // The legacy import can be replayed after restart; the module returns its
    // existing rows and unchanged grants without duplicating durable state.
    let repeated = history_import::run(&world.engine, &world.fixture.proxy)
        .await
        .unwrap()
        .expect("the legacy file is still present");
    assert_eq!(
        (repeated.history, repeated.trust, repeated.skipped),
        (2, 1, 0)
    );
    let history = world.engine.history(50).await.unwrap();
    assert_eq!(history.len(), initial_history_count);
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_flow_trust_the_module_holds_approves_that_tool_in_that_flow_after_a_restart() {
    let world = world().await;
    let tenant = crate::core::runtime::Tenant::default();
    world
        .engine
        .grant_flow_trust(&tenant, "flow-1", "http_request")
        .await
        .expect("the module keeps the flow trust");
    // A fresh engine over the same storage: nothing in memory carries over.
    let again = NativeEngine::on(world.config.clone(), world.fixture.proxy.clone());
    assert_eq!(
        again.flow_trust(&tenant, "flow-1").await.unwrap(),
        ["http_request"]
    );
    let shown = Arc::new(Shown::default());
    let mut in_flow = input("http_request", &shown);
    in_flow.variant = Variant::FlowStrict;
    in_flow.flow = Some("flow-1".into());
    assert!(matches!(
        again.check(in_flow).await.unwrap(),
        Checked::Approved { .. }
    ));
    assert!(again
        .revoke_flow_trust(&tenant, "flow-1", "http_request")
        .await
        .unwrap());
    let mut revoked = input("http_request", &shown);
    revoked.variant = Variant::FlowStrict;
    revoked.flow = Some("flow-1".into());
    assert!(matches!(
        again.check(revoked).await.unwrap(),
        Checked::Pending { .. }
    ));
}

#[path = "engine_origin_tests.rs"]
mod origin_tests;
