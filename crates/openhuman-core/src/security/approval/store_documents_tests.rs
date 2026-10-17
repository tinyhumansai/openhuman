use super::*;
use chrono::{Duration, Timelike};
use tinystoragedrivers::{MemoryStorage, Scope, StorageBackend};

fn docs_in(storage: &MemoryStorage, scope: &str) -> Docs {
    Docs::new(&storage.for_scope(&Scope::new(scope).unwrap()).unwrap())
}

fn docs() -> Docs {
    docs_in(&MemoryStorage::new(), "local")
}

fn pending(id: &str, created_secs_ago: i64, expires_in: Option<i64>) -> PendingApproval {
    let now = Utc::now();
    PendingApproval {
        request_id: id.to_string(),
        tool_name: "shell".to_string(),
        action_summary: format!("run {id}"),
        args_redacted: json!({ "file.name": "x", "cmd": "ls" }),
        created_at: now - Duration::seconds(created_secs_ago),
        expires_at: expires_in.map(|secs| now + Duration::seconds(secs)),
        source_context: Some(ApprovalSourceContext::Flow {
            flow_id: "flow-1".to_string(),
            run_id: "run-1".to_string(),
            node_id: None,
        }),
        tool_call_id: Some(format!("call-{id}")),
        agent_id: None,
    }
}

#[test]
fn pending_requests_round_trip_oldest_first() {
    let store = docs();
    store.insert_pending(&pending("b", 10, None), "s1").unwrap();
    store.insert_pending(&pending("a", 20, None), "s1").unwrap();
    let listed = store.list_pending().unwrap();
    let ids: Vec<&str> = listed.iter().map(|p| p.request_id.as_str()).collect();
    assert_eq!(ids, ["a", "b"]);
    assert_eq!(listed[0].args_redacted["file.name"], json!("x"));
    assert_eq!(listed[0].tool_call_id.as_deref(), Some("call-a"));
    assert!(matches!(
        listed[0].source_context,
        Some(ApprovalSourceContext::Flow { .. })
    ));
    assert!(
        store.insert_pending(&pending("a", 0, None), "s1").is_err(),
        "a request id is inserted once"
    );
}

#[test]
fn a_request_is_decided_exactly_once() {
    let store = docs();
    store.insert_pending(&pending("r", 0, None), "s").unwrap();
    assert_eq!(store.get_decision("r").unwrap(), None);
    let decided = store.decide("r", ApprovalDecision::ApproveOnce).unwrap();
    assert_eq!(decided.map(|p| p.request_id), Some("r".to_string()));
    assert!(store.decide("r", ApprovalDecision::Deny).unwrap().is_none());
    assert_eq!(
        store.get_decision("r").unwrap(),
        Some(ApprovalDecision::ApproveOnce)
    );
    assert!(store
        .decide("missing", ApprovalDecision::Deny)
        .unwrap()
        .is_none());
    assert!(store.list_pending().unwrap().is_empty());
}

#[test]
fn concurrent_deciders_never_both_win() {
    let storage = MemoryStorage::new();
    let store = docs_in(&storage, "local");
    store
        .insert_pending(&pending("race", 0, None), "s")
        .unwrap();
    let winners: usize = (0..8)
        .map(|_| {
            let store = store.clone();
            std::thread::spawn(move || {
                store
                    .decide("race", ApprovalDecision::ApproveOnce)
                    .unwrap()
                    .is_some()
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|handle| usize::from(handle.join().unwrap()))
        .sum();
    assert_eq!(winners, 1);
}

#[test]
fn stale_requests_expire_to_deny_once() {
    let store = docs();
    store
        .insert_pending(&pending("old", 0, Some(-5)), "s")
        .unwrap();
    store
        .insert_pending(&pending("fresh", 0, Some(600)), "s")
        .unwrap();
    store
        .insert_pending(&pending("forever", 0, None), "s")
        .unwrap();
    let expired = store.expire_stale(Utc::now()).unwrap();
    assert_eq!(
        expired
            .iter()
            .map(|p| p.request_id.as_str())
            .collect::<Vec<_>>(),
        ["old"]
    );
    assert!(
        store.expire_stale(Utc::now()).unwrap().is_empty(),
        "only once"
    );
    assert_eq!(
        store.get_decision("old").unwrap(),
        Some(ApprovalDecision::Deny)
    );
    let left: Vec<String> = store
        .list_pending()
        .unwrap()
        .into_iter()
        .map(|p| p.request_id)
        .collect();
    assert_eq!(left.len(), 2);
    assert!(!left.contains(&"old".to_string()));
}

#[test]
fn execution_is_recorded_after_a_decision_and_only_once() {
    let store = docs();
    store.insert_pending(&pending("r", 0, None), "s").unwrap();
    assert!(
        !store
            .record_execution("r", ExecutionOutcome::Success, None)
            .unwrap(),
        "not before a decision"
    );
    store.decide("r", ApprovalDecision::ApproveOnce).unwrap();
    let long = format!("token=sk-abcdefghijklmnopqrstuvwxyz {}", "e".repeat(800));
    assert!(store
        .record_execution("r", ExecutionOutcome::Failure, Some(&long))
        .unwrap());
    assert!(
        !store
            .record_execution("r", ExecutionOutcome::Success, None)
            .unwrap(),
        "the first outcome wins"
    );
    assert!(!store
        .record_execution("missing", ExecutionOutcome::Success, None)
        .unwrap());
    let stored = store
        .run(|docs| async move { docs.get(APPROVALS, "r").await })
        .unwrap()
        .unwrap();
    assert_eq!(stored.doc["execution_outcome"], json!("failure"));
    let error = stored.doc["execution_error"].as_str().unwrap();
    assert!(error.chars().count() <= 512, "capped");
    assert!(
        !error.contains("sk-abcdefghijklmnopqrstuvwxyz"),
        "the secret is redacted before storage: {error}"
    );
}

#[test]
fn recent_decisions_are_newest_first_and_capped() {
    let store = docs();
    let base = Utc::now();
    for (n, id) in ["a", "b", "c"].into_iter().enumerate() {
        store.insert_pending(&pending(id, 0, None), "s").unwrap();
        // Distinct sub-second stamps (the fixed-width format must order them).
        let at = base + Duration::milliseconds(n as i64 * 100);
        store
            .decide_at(id, ApprovalDecision::ApproveOnce, at)
            .unwrap();
    }
    store
        .insert_pending(&pending("open", 0, None), "s")
        .unwrap();
    let recent = store.list_recent_decisions(2).unwrap();
    assert_eq!(
        recent
            .iter()
            .map(|e| e.request_id.as_str())
            .collect::<Vec<_>>(),
        ["c", "b"]
    );
    assert_eq!(recent[0].decision, ApprovalDecision::ApproveOnce);
}

#[test]
fn purging_a_session_drops_only_its_undecided_requests() {
    let store = docs();
    store
        .insert_pending(&pending("mine", 0, None), "s1")
        .unwrap();
    store
        .insert_pending(&pending("decided", 0, None), "s1")
        .unwrap();
    store.decide("decided", ApprovalDecision::Deny).unwrap();
    store
        .insert_pending(&pending("theirs", 0, None), "s2")
        .unwrap();
    assert_eq!(store.purge_session("s1").unwrap(), 1);
    assert!(store.get_decision("decided").unwrap().is_some());
    let left: Vec<String> = store
        .list_pending()
        .unwrap()
        .into_iter()
        .map(|p| p.request_id)
        .collect();
    assert_eq!(left, ["theirs"]);
}

#[test]
fn a_preauthorization_is_audit_only() {
    let store = docs();
    store
        .insert_decided(
            "http",
            "pre-authorized",
            "s",
            &ApprovalSourceContext::Flow {
                flow_id: "flow-1".to_string(),
                run_id: String::new(),
                node_id: None,
            },
            ApprovalDecision::ApproveAlwaysForFlow,
        )
        .unwrap();
    assert!(store.list_pending().unwrap().is_empty());
    let audit = store.list_recent_decisions(10).unwrap();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].decision, ApprovalDecision::ApproveAlwaysForFlow);
}

#[test]
fn flow_trust_grants_list_and_revoke() {
    let store = docs();
    store.insert_flow_trust("flow-1", "shell").unwrap();
    store.insert_flow_trust("flow-1", "shell").unwrap();
    store.insert_flow_trust("flow-1", "http").unwrap();
    store.insert_flow_trust("flow-2", "shell").unwrap();
    assert_eq!(store.list_flow_trust("flow-1").unwrap(), ["http", "shell"]);
    assert!(store.is_flow_tool_trusted("flow-1", "shell").unwrap());
    assert!(!store.is_flow_tool_trusted("flow-1", "mail").unwrap());
    assert_eq!(
        store
            .delete_flow_trust("flow-1", Some(&["shell".to_string(), "nope".to_string()]))
            .unwrap(),
        1
    );
    assert_eq!(store.delete_flow_trust("flow-1", None).unwrap(), 1);
    assert!(store.list_flow_trust("flow-1").unwrap().is_empty());
    assert!(store.is_flow_tool_trusted("flow-2", "shell").unwrap());
    assert_ne!(trust_id("a/b", "c"), trust_id("a", "b/c"));
}

#[test]
fn scopes_keep_users_apart() {
    let storage = MemoryStorage::new();
    let alice = docs_in(&storage, "alice");
    let bob = docs_in(&storage, "bob");
    alice.insert_pending(&pending("r", 0, None), "s").unwrap();
    alice.insert_flow_trust("flow-1", "shell").unwrap();
    assert!(bob.list_pending().unwrap().is_empty());
    assert!(bob
        .decide("r", ApprovalDecision::ApproveOnce)
        .unwrap()
        .is_none());
    assert!(!bob.is_flow_tool_trusted("flow-1", "shell").unwrap());
    assert_eq!(alice.list_pending().unwrap().len(), 1);
}

#[test]
fn an_unreadable_source_context_reads_as_absent() {
    let store = docs();
    store.insert_pending(&pending("r", 0, None), "s").unwrap();
    store
        .run(|docs| async move {
            let mut stored = docs.get(APPROVALS, "r").await?.unwrap();
            stored.doc["source_context"] = json!("not json");
            docs.put(APPROVALS, "r", stored.doc, Precondition::None)
                .await
                .map(|_| ())
        })
        .unwrap();
    assert!(store.list_pending().unwrap()[0].source_context.is_none());
}

#[test]
fn a_missing_error_stays_missing() {
    assert!(audit_error(None).is_none());
    assert_eq!(audit_error(Some("plain")).as_deref(), Some("plain"));
}

#[test]
fn sub_second_timestamps_order_and_expire_on_time() {
    let store = docs();
    let base = Utc::now();
    // Whole-second and sub-second values in the same second must sort by time.
    let whole = base.with_nanosecond(0).unwrap();
    for (id, at) in [("x", whole + Duration::milliseconds(500)), ("w", whole)] {
        let mut p = pending(id, 0, None);
        p.created_at = at;
        store.insert_pending(&p, "s").unwrap();
    }
    let ids: Vec<String> = store
        .list_pending()
        .unwrap()
        .into_iter()
        .map(|p| p.request_id)
        .collect();
    assert_eq!(ids, ["w", "x"]);

    // An approval due 900 ms into a second is not expired at 100 ms.
    let due = whole + Duration::milliseconds(900);
    let mut p = pending("late", 0, None);
    p.expires_at = Some(due);
    store.insert_pending(&p, "s").unwrap();
    assert!(store
        .expire_stale(whole + Duration::milliseconds(100))
        .unwrap()
        .is_empty());
    assert_eq!(store.expire_stale(due).unwrap().len(), 1);
}

#[test]
fn a_captured_store_decides_without_the_task_scope() {
    let store = docs();
    store.insert_pending(&pending("r", 0, None), "s").unwrap();
    let decided = crate::security::approval::store::decide_captured(
        &Ok(store.clone()),
        "r",
        ApprovalDecision::Deny,
    )
    .unwrap();
    assert_eq!(decided.map(|p| p.request_id), Some("r".to_string()));
}

#[test]
fn an_unresolved_scope_fails_the_captured_decide_instead_of_using_another_scope() {
    let captured = Err("no acting agent".to_string());
    let error =
        crate::security::approval::store::decide_captured(&captured, "r", ApprovalDecision::Deny)
            .unwrap_err();
    assert!(error.to_string().contains("unresolved"), "{error}");
}

#[test]
fn expiry_is_compared_in_epoch_milliseconds() {
    let store = docs();
    let whole = Utc::now().with_nanosecond(0).unwrap();
    let due = whole + Duration::milliseconds(3);
    let mut p = pending("tight", 0, None);
    p.expires_at = Some(due);
    store.insert_pending(&p, "s").unwrap();
    assert!(store
        .expire_stale(whole + Duration::milliseconds(2))
        .unwrap()
        .is_empty());
    assert_eq!(store.expire_stale(due).unwrap().len(), 1);
}

#[test]
fn the_parking_agent_round_trips_and_names_the_pending_owner() {
    let store = docs();
    let mut owned = pending("owned", 10, None);
    owned.agent_id = Some("alpha".to_string());
    store.insert_pending(&owned, "s1").unwrap();
    store
        .insert_pending(&pending("process", 5, None), "s1")
        .unwrap();

    let listed = store.list_pending().unwrap();
    assert_eq!(listed[0].agent_id.as_deref(), Some("alpha"));
    assert_eq!(listed[1].agent_id, None);
    assert_eq!(
        store.pending_agent("owned").unwrap(),
        Some(Some("alpha".to_string()))
    );
    assert_eq!(store.pending_agent("process").unwrap(), Some(None));
    assert_eq!(store.pending_agent("missing").unwrap(), None);

    store.decide("owned", ApprovalDecision::Deny).unwrap();
    assert_eq!(store.pending_agent("owned").unwrap(), None);
}
