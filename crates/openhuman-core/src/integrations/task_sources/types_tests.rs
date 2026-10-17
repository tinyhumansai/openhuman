use super::*;
use serde_json::json;

#[test]
fn task_source_serializes_camel_case() {
    let src = TaskSource {
        id: "s1".into(),
        provider: ProviderSlug::Linear,
        connection_id: None,
        name: Some("My Linear".into()),
        enabled: true,
        filter: FilterSpec::Linear {
            team_id: Some("team-1".into()),
            assignee_is_me: true,
            state: None,
            extra: json!({}),
        },
        interval_secs: 1800,
        target: SourceTarget::TodoOnly,
        max_tasks_per_fetch: 25,
        created_at: DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
        last_fetch_at: None,
        last_status: None,
    };
    let v = serde_json::to_value(&src).unwrap();
    assert_eq!(v["maxTasksPerFetch"], 25);
    assert_eq!(v["intervalSecs"], 1800);
    assert_eq!(v["target"], "todo_only");
    // connection_id / last_fetch_at omitted when None.
    assert!(v.get("connectionId").is_none());
    let back: TaskSource = serde_json::from_value(v).unwrap();
    assert_eq!(back, src);
}

#[test]
fn provider_types_are_the_connector_contracts() {
    // The wire tags are the contract's (`tinyconnectors_bus::vocabulary`), pinned
    // there; the host only checks it still names the same types, so a stored
    // source written before the move reads back unchanged.
    let stored = r#"{"provider":"github","repo":"o/n","labels":[],"assignee_is_me":false}"#;
    let filter: FilterSpec = serde_json::from_str(stored).unwrap();
    assert_eq!(filter.provider(), ProviderSlug::Github);
    assert_eq!(SourceTarget::default(), SourceTarget::AgentTodoProactive);
    assert_eq!(FetchReason::Manual.as_str(), "manual");
}
