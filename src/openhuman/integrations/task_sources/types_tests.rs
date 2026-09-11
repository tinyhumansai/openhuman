use super::*;
use serde_json::json;

#[test]
fn provider_slug_roundtrips() {
    for p in [
        ProviderSlug::Github,
        ProviderSlug::Notion,
        ProviderSlug::Linear,
        ProviderSlug::Clickup,
    ] {
        assert_eq!(ProviderSlug::parse(p.as_str()).unwrap(), p);
    }
    assert_eq!(ProviderSlug::parse("GitHub").unwrap(), ProviderSlug::Github);
    assert!(ProviderSlug::parse("jira").is_err());
}

#[test]
fn provider_slug_serde_is_snake_case() {
    assert_eq!(
        serde_json::to_string(&ProviderSlug::Clickup).unwrap(),
        "\"clickup\""
    );
}

#[test]
fn filter_spec_tagged_by_provider() {
    let f = FilterSpec::Github {
        repo: Some("owner/name".into()),
        labels: vec!["bug".into()],
        assignee_is_me: true,
        state: Some("open".into()),
        fetch_mode: Default::default(),
        extra: json!({}),
    };
    let s = serde_json::to_value(&f).unwrap();
    assert_eq!(s["provider"], "github");
    assert_eq!(s["repo"], "owner/name");
    assert_eq!(f.provider(), ProviderSlug::Github);

    let back: FilterSpec = serde_json::from_value(s).unwrap();
    assert_eq!(back, f);
}

#[test]
fn notion_filter_roundtrips_with_board() {
    let f = FilterSpec::Notion {
        database_id: Some("db-1".into()),
        assigned_to_me: true,
        status: Some("In Progress".into()),
        extra: json!({"page_size": 10}),
    };
    let back: FilterSpec = serde_json::from_value(serde_json::to_value(&f).unwrap()).unwrap();
    assert_eq!(back, f);
    assert_eq!(back.provider(), ProviderSlug::Notion);
}

#[test]
fn source_target_defaults_to_proactive() {
    assert_eq!(SourceTarget::default(), SourceTarget::AgentTodoProactive);
}

#[test]
fn fetch_reason_as_str() {
    assert_eq!(FetchReason::Periodic.as_str(), "periodic");
    assert_eq!(
        FetchReason::ConnectionCreated.as_str(),
        "connection_created"
    );
    assert_eq!(FetchReason::Manual.as_str(), "manual");
}

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
        assigned_executor: None,
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
