use super::*;
use tinysecurity_bus::{
    ApprovalBinding2, AuditCorrelation2, AuditEvent2, AuditKind2, CallerContext, Origin,
};
use tinystoragedrivers::{Scope, StorageBackend};

fn rows() -> Rows {
    let storage = tinystoragedrivers_testkit::memory_backend();
    let scoped = storage.for_scope(&Scope::new("local").unwrap()).unwrap();
    Rows::over(Arc::clone(scoped.documents()))
}

fn handle(agent: &str) -> SubjectScope {
    SubjectScope {
        user: "u".repeat(64),
        workspace: "w".repeat(64),
        agent: agent.repeat(64),
    }
}

fn row(id: &str, version: u64, state: ApprovalLifecycle2, scope: &SubjectScope) -> ApprovalRow2 {
    ApprovalRow2 {
        request_id: id.repeat(32)[..32].to_owned(),
        version,
        binding: ApprovalBinding2 {
            binding_version: 1,
            scope: scope.clone(),
            policy_ref: "p".repeat(64),
            generation: 1,
            call_sha256: "c".repeat(64),
            content_sha256: "d".repeat(64),
            origin: Origin::Operator,
            caller: CallerContext::default(),
        },
        host_instance: "e".repeat(32),
        nonce: "f".repeat(32),
        tool_name: "gmail_send".into(),
        session_id: None,
        audit_error: None,
        external_effect: true,
        args_redacted: "{}".into(),
        summary: "send".into(),
        created_at_ms: 1_000,
        expires_at_ms: 601_000,
        state,
        choice: None,
        attribution: None,
        decision_ticket_sha256: None,
        outcome: None,
        reservation_id: None,
        legacy_history_json: None,
        judge: None,
        audit: AuditCorrelation2 {
            event_id: "a".repeat(32),
            expected_tail: 0,
            generation: 1,
        },
    }
}

fn record(row: &ApprovalRow2) -> ApprovalRecord2 {
    ApprovalRecord2 {
        record_id: row.request_id.clone(),
        version: row.version,
        policy_id: row.binding.policy_ref.clone(),
        generation: row.binding.generation,
        fingerprint: "9".repeat(32),
        record_json: serde_json::to_string(row).unwrap(),
    }
}

fn cas(row: &ApprovalRow2, expected: Option<u64>, mutation: &str) -> ApprovalCasRequest2 {
    ApprovalCasRequest2 {
        scope: row.binding.scope.clone(),
        record_id: row.request_id.clone(),
        expected_version: expected,
        next_record: record(row),
        mutation_id: mutation.repeat(32)[..32].to_owned(),
    }
}

fn pending(scope: &SubjectScope) -> ApprovalRow2 {
    row("1", 1, ApprovalLifecycle2::Pending, scope)
}

#[tokio::test]
async fn a_row_is_created_once_and_advanced_only_from_the_expected_version() {
    let store = rows();
    let scope = handle("a");
    let first = pending(&scope);
    let receipt = store.cas(&cas(&first, None, "a")).await.unwrap();
    assert_eq!(receipt.resulting_version, 1);
    assert!(
        store.cas(&cas(&first, None, "b")).await.is_err(),
        "create-only refuses an existing row"
    );

    let mut second = first.clone();
    second.version = 2;
    second.state = ApprovalLifecycle2::Denied;
    assert!(
        store.cas(&cas(&second, Some(7), "c")).await.is_err(),
        "a stale expected version is a conflict"
    );
    let advanced = store.cas(&cas(&second, Some(1), "d")).await.unwrap();
    assert_eq!(advanced.resulting_version, 2);
    let stored = store.row(&first.request_id).await.unwrap().unwrap();
    assert_eq!(stored.row.state, ApprovalLifecycle2::Denied);
    assert!(
        stored.decided_ms.is_some(),
        "leaving pending stamps the decision time"
    );
}

#[tokio::test]
async fn a_retried_mutation_returns_its_original_receipt_even_after_later_versions() {
    let store = rows();
    let scope = handle("a");
    let first = pending(&scope);
    let created = cas(&first, None, "a");
    let original = store.cas(&created).await.unwrap();

    let mut second = first.clone();
    second.version = 2;
    second.state = ApprovalLifecycle2::Cancelled;
    store.cas(&cas(&second, Some(1), "b")).await.unwrap();

    // The module retries the create after the row moved on: the first receipt, not a conflict.
    assert_eq!(store.cas(&created).await.unwrap(), original);
    assert_eq!(
        store
            .row(&first.request_id)
            .await
            .unwrap()
            .unwrap()
            .row
            .version,
        2,
        "replaying an old mutation does not roll the row back"
    );
}

#[tokio::test]
async fn a_mutation_identity_cannot_carry_other_content() {
    let store = rows();
    let scope = handle("a");
    let first = pending(&scope);
    store.cas(&cas(&first, None, "a")).await.unwrap();
    let mut changed = first.clone();
    changed.summary = "something else".into();
    assert!(store.cas(&cas(&changed, None, "a")).await.is_err());
}

#[tokio::test]
async fn an_interrupted_mutation_is_completed_by_its_retry() {
    let store = rows();
    let scope = handle("a");
    let first = pending(&scope);
    let request = cas(&first, None, "a");
    // The intent was recorded but the row never written (a crash between the two).
    store
        .docs
        .put(
            MUTATIONS,
            &digest_id(&[&scope_key(&request.scope), &request.mutation_id]),
            json!({
                "record_id": request.record_id,
                "record_json": request.next_record.record_json,
                "version": 1,
            }),
            Precondition::Absent,
        )
        .await
        .unwrap();
    assert!(store.row(&first.request_id).await.unwrap().is_none());
    let receipt = store.cas(&request).await.unwrap();
    assert_eq!(receipt.resulting_version, 1);
    assert!(store.row(&first.request_id).await.unwrap().is_some());
}

#[tokio::test]
async fn load_answers_one_subject_newest_first_and_never_more_than_the_limit() {
    let store = rows();
    let mine = handle("a");
    let theirs = handle("b");
    let mut written = Vec::new();
    for (index, id) in ["1", "2", "3"].iter().enumerate() {
        let mut stored = row(id, 1, ApprovalLifecycle2::Pending, &mine);
        stored.created_at_ms += index as u64;
        store.cas(&cas(&stored, None, id)).await.unwrap();
        written.push(record(&stored));
        tokio::time::sleep(std::time::Duration::from_millis(3)).await;
    }
    let other = row("4", 1, ApprovalLifecycle2::Pending, &theirs);
    store.cas(&cas(&other, None, "4")).await.unwrap();

    let loaded = store.load(&mine).await.unwrap();
    let ids: Vec<&str> = loaded.iter().map(|r| &r.record_id[..1]).collect();
    assert_eq!(ids, ["3", "2", "1"]);
    // The module gets back exactly the envelope it wrote.
    written.reverse();
    assert_eq!(loaded, written);
    assert_eq!(store.load(&theirs).await.unwrap().len(), 1);
    const {
        assert!(LOAD_LIMIT <= 1024, "the module refuses a larger answer");
    }
}

fn event(sequence: u64, id: &str) -> AuditEvent2 {
    AuditEvent2 {
        sequence,
        event_id: id.repeat(32)[..32].to_owned(),
        generation: 1,
        kind: AuditKind2::Approval,
        attribution: "pending".into(),
        arguments_json: "{}".into(),
        rule_ids: vec![],
    }
}

fn batch(expected_tail: u64, events: Vec<AuditEvent2>) -> AuditSinkRequest2 {
    AuditSinkRequest2 {
        scope: handle("a"),
        stream_id: "5".repeat(32),
        expected_tail,
        events,
    }
}

#[tokio::test]
async fn the_audit_sink_accepts_contiguous_batches_and_replays_idempotently() {
    let store = rows();
    let first = batch(0, vec![event(1, "1")]);
    let receipt = store.append(&first).await.unwrap();
    assert_eq!(receipt.new_tail, 1);
    first.verify_receipt(&receipt).unwrap();
    assert_eq!(store.tail(&first.stream_id).await.unwrap(), 1);

    let second = batch(1, vec![event(2, "2"), event(3, "3")]);
    assert_eq!(store.append(&second).await.unwrap().new_tail, 3);

    // An exact replay of an earlier batch is the same receipt and moves nothing.
    assert_eq!(store.append(&first).await.unwrap(), receipt);
    assert_eq!(store.tail(&first.stream_id).await.unwrap(), 3);
}

#[tokio::test]
async fn the_audit_sink_refuses_a_gap_and_a_different_event_at_a_taken_sequence() {
    let store = rows();
    store.append(&batch(0, vec![event(1, "1")])).await.unwrap();
    assert!(
        store.append(&batch(5, vec![event(6, "6")])).await.is_err(),
        "a gap after the committed tail"
    );
    assert_eq!(store.tail(&"5".repeat(32)).await.unwrap(), 1);
    assert!(
        store.append(&batch(0, vec![event(1, "9")])).await.is_err(),
        "another event at a committed sequence"
    );
    assert!(
        store.append(&batch(0, vec![])).await.is_err(),
        "an empty batch is malformed"
    );
}

#[tokio::test]
async fn a_subjects_audit_stream_is_created_once_and_kept() {
    let store = rows();
    let first = store.stream_for("subject-a").await.unwrap();
    assert_eq!(first.len(), 32);
    assert_eq!(store.stream_for("subject-a").await.unwrap(), first);
    assert_ne!(store.stream_for("subject-b").await.unwrap(), first);
}

#[tokio::test]
async fn what_the_user_chose_is_kept_beside_the_row() {
    let store = rows();
    assert_eq!(store.label("r").await.unwrap(), None);
    store
        .put_label("r", "approve_always_for_tool")
        .await
        .unwrap();
    assert_eq!(
        store.label("r").await.unwrap().as_deref(),
        Some("approve_always_for_tool")
    );
}

#[tokio::test]
async fn decided_and_stale_queries_split_the_rows_by_their_state() {
    let store = rows();
    let scope = handle("a");
    let mut expired_pending = pending(&scope);
    expired_pending.expires_at_ms = 2_000;
    store.cas(&cas(&expired_pending, None, "a")).await.unwrap();
    let mut live = row("2", 1, ApprovalLifecycle2::Pending, &scope);
    live.expires_at_ms = u64::MAX / 4;
    store.cas(&cas(&live, None, "b")).await.unwrap();
    let mut decided = row("3", 1, ApprovalLifecycle2::Denied, &scope);
    decided.choice = Some(tinysecurity_bus::ApprovalChoice2::Deny);
    store.cas(&cas(&decided, None, "c")).await.unwrap();

    let stale = store.stale(10_000, 10).await.unwrap();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].row.request_id, expired_pending.request_id);
    let history = store.decided(10).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].row.request_id, decided.request_id);
}

#[test]
fn identifiers_are_short_stable_and_distinguish_their_parts() {
    assert_eq!(digest_id(&["a", "bc"]).len(), 32);
    assert_eq!(digest_id(&["a", "bc"]), digest_id(&["a", "bc"]));
    assert_ne!(digest_id(&["a", "bc"]), digest_id(&["ab", "c"]));
}
