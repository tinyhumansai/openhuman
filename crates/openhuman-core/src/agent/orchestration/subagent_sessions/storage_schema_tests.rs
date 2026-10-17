use super::*;
use serde_json::json;
use tinystoragedrivers::Meta;

fn source(id: &str, body: Value) -> SourceRecord {
    SourceRecord {
        id: id.to_string(),
        body,
        meta: Meta::unknown(),
    }
}

#[test]
fn a_camel_case_session_becomes_a_snake_case_body_with_millisecond_times() {
    let out = sessions_v2(vec![source(
        "s1",
        json!({
            "subagentSessionId": "s1", "parentSession": "p", "parentThreadId": "t",
            "workerThreadId": null, "agentId": "researcher", "displayName": null,
            "model": null, "sandboxMode": "workspace", "actionRoot": null,
            "taskKey": "task", "taskTitle": "Task", "currentTaskId": null,
            "status": "idle", "reusable": true, "latestHistory": [{"role": "user"}],
            "latestError": null,
            "createdAt": "1970-01-01T00:00:01Z", "created_at": "1970-01-01T00:00:01Z",
            "updatedAt": "1970-01-01T00:00:02Z", "lastUsedAt": "1970-01-01T00:00:03Z",
        }),
    )])
    .unwrap();
    assert_eq!(out[0].collection, SESSIONS);
    assert_eq!(out[0].id, "s1");
    assert_eq!(
        out[0].body,
        json!({
            "subagent_session_id": "s1", "parent_session": "p", "parent_thread_id": "t",
            "agent_id": "researcher", "sandbox_mode": "workspace",
            "task_key": "task", "task_title": "Task", "status": "idle", "reusable": true,
            "latest_history": [{"role": "user"}],
            "created_ms": 1000, "updated_ms": 2000, "last_used_ms": 3000,
        })
    );
}

#[test]
fn the_schema_composes_under_its_owner() {
    tinystoragedrivers::schema::Registry::compose([storage_schema()]).unwrap();
}
