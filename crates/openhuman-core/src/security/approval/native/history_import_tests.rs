use super::*;
use chrono::{Duration, Utc};
use tinybus::Proxy;

fn legacy(id: &str, decision: Option<&str>, executed: bool, age_minutes: i64) -> LegacyApproval {
    let created = Utc::now() - Duration::minutes(age_minutes);
    LegacyApproval {
        request_id: id.into(),
        tool_name: "composio".into(),
        agent_id: None,
        created_at: created,
        expires_at: Some(created + Duration::minutes(10)),
        decided_at: decision.map(|_| created + Duration::seconds(5)),
        decision: decision.map(str::to_owned),
        executed,
    }
}

#[test]
fn a_legacy_decision_maps_to_the_module_state_it_ended_in() {
    assert_eq!(
        state_of(&legacy("a", Some("approve_once"), false, 1)),
        ApprovalState::Allowed
    );
    assert_eq!(
        state_of(&legacy("a", Some("approve_always_for_tool"), true, 1)),
        ApprovalState::Executed
    );
    assert_eq!(
        state_of(&legacy("a", Some("deny"), false, 1)),
        ApprovalState::Denied
    );
    // A request an earlier launch left undecided can never be resumed.
    assert_eq!(
        state_of(&legacy("a", None, false, 1)),
        ApprovalState::Expired
    );
}

#[test]
fn the_legacy_record_carries_identity_without_the_arguments_or_summary() {
    let row = legacy("req-1", Some("deny"), false, 3);
    let record = legacy_record(&row);
    assert_eq!(record.request_id, "req-1");
    assert_eq!(record.caller.agent, "operator");
    assert_eq!(record.caller.call_id, "req-1");
    assert_eq!(record.fingerprint.len(), 64);
    assert_eq!(record.state, ApprovalState::Denied);
    assert_eq!(
        record.expires_at_ms,
        u64::try_from(row.expires_at.unwrap().timestamp_millis()).unwrap()
    );
    let json = serde_json::to_string(&record).unwrap();
    assert!(
        !json.contains("composio"),
        "the tool name is not sent: {json}"
    );
    // The same row always fingerprints the same, so a repeat run is recognized.
    assert_eq!(legacy_record(&row), record);
    let mut other = row.clone();
    other.request_id = "req-2".into();
    assert_ne!(legacy_record(&other).fingerprint, record.fingerprint);
}

#[test]
fn an_agent_scoped_row_keeps_its_agent_and_a_row_without_expiry_gets_the_default_window() {
    let mut row = legacy("req-1", Some("approve_once"), false, 3);
    row.agent_id = Some("agent-7".into());
    row.expires_at = None;
    let record = legacy_record(&row);
    assert_eq!(record.caller.agent, "agent-7");
    assert_eq!(
        record.expires_at_ms,
        u64::try_from(row.created_at.timestamp_millis()).unwrap() + DEFAULT_WINDOW_MS
    );
}

#[test]
fn only_the_most_recent_decided_rows_are_carried_over_oldest_first() {
    let mut history = LegacyHistory::default();
    history
        .approvals
        .push(legacy("undecided", None, false, 100_000));
    let total = HISTORY_LIMIT + 7;
    for index in 0..total {
        let age = i64::try_from(total - index).unwrap();
        history.approvals.push(legacy(
            &format!("r{index}"),
            Some("approve_once"),
            false,
            age,
        ));
    }
    let carried = recent_decided(&history);
    assert_eq!(carried.len(), HISTORY_LIMIT);
    assert_eq!(carried.first().unwrap().request_id, "r7");
    assert_eq!(
        carried.last().unwrap().request_id,
        format!("r{}", total - 1)
    );
    assert!(carried.iter().all(|row| row.request_id != "undecided"));
}

fn workspace() -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    (dir, config)
}

async fn absent_module() -> (
    tinybus::Connection,
    Proxy,
    tinybus::module::ModuleObjectClient,
) {
    use tinybus::transport::memory::MemoryBus;
    let transport = MemoryBus::new();
    tinybus::broker::Broker::new().spawn(transport.clone());
    let host = tinybus::Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    let proxy = host
        .proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)
        .unwrap()
        .with_timeout(std::time::Duration::from_millis(200));
    let object = crate::modules::ops::fixture_client(proxy.clone())
        .with_timeout(std::time::Duration::from_millis(200))
        .object_client(crate::modules::security::MODULE_ID, names::OBJECT_PATH)
        .await
        .unwrap();
    (host, proxy, object)
}

fn seed_legacy(config: &Config) {
    use crate::storage::{DocumentStoreExt, Precondition};
    let opened = crate::storage::local::open("history import test", &config.workspace_dir).unwrap();
    let storage = opened.scoped().unwrap();
    let docs = std::sync::Arc::clone(storage.documents());
    let now = Utc::now();
    let history = serde_json::json!({
        "tool_name": "composio",
        "action_summary": "s",
        "args_redacted": "{}",
        "session_id": "legacy-session",
        "created_ms": now.timestamp_millis(),
        "deadline_ms": (now + Duration::minutes(10)).timestamp_millis(),
        "decided_ms": (now + Duration::seconds(1)).timestamp_millis(),
        "decision": "approve_once",
        "pending": false,
    });
    crate::storage::block_on(async move {
        for spec in crate::security::approval::collection_specs() {
            docs.ensure_collection(&spec).await?;
        }
        docs.put(
            crate::security::approval::storage_schema::APPROVALS,
            "r1",
            history,
            Precondition::Absent,
        )
        .await
    })
    .unwrap();
}

#[tokio::test]
async fn a_workspace_with_no_legacy_history_has_nothing_to_import() {
    let (_dir, config) = workspace();
    let (_host, proxy, object) = absent_module().await;
    let engine = NativeEngine::on(config.clone(), proxy);
    let object = crate::modules::security::SecurityProxy::module(object);
    assert_eq!(run(&engine, &object).await, Ok(None));
    assert!(read(config).await.unwrap().is_none());
}

#[tokio::test]
async fn a_legacy_import_that_cannot_reach_the_module_remains_retryable() {
    let (_dir, config) = workspace();
    seed_legacy(&config);
    let (_host, proxy, object) = absent_module().await;
    let engine = NativeEngine::on(config.clone(), proxy);
    let object = crate::modules::security::SecurityProxy::module(object);
    let result = run(&engine, &object).await;
    assert!(
        matches!(result, Err(EngineError::Unavailable(_))),
        "{result:?}"
    );
    assert_eq!(
        read(config.clone()).await.unwrap().unwrap().approvals.len(),
        1
    );
}
