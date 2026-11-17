use super::*;
use serde_json::json;
use tempfile::TempDir;

fn config(tmp: &TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().to_path_buf(),
        ..Config::default()
    }
}

fn graph() -> serde_json::Value {
    json!({
        "nodes": [{"id":"trigger","name":"Trigger","kind":"trigger","config":{"trigger_kind":"manual"}}],
        "edges": []
    })
}

#[test]
fn catalog_clients_preserve_definition_revisions_and_suggestion_semantics() {
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    let flow = create_flow(&config, "before".into(), graph(), false, true)
        .unwrap()
        .workflow;
    assert_eq!(get_flow(&config, &flow.id).unwrap().unwrap().name, "before");
    let (listed, skipped) = list_flows(&config).unwrap();
    assert_eq!(skipped, 0);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, flow.id);

    let updated = update_flow_graph(
        &config,
        &flow.id,
        "after".into(),
        graph(),
        true,
        None,
        false,
        Some(&flow.updated_at),
    )
    .unwrap()
    .workflow;
    assert_eq!(updated.name, "after");
    assert_eq!(list_revisions(&config, &flow.id, 10).unwrap().len(), 1);
    assert!(matches!(
        update_flow_graph(
            &config,
            &flow.id,
            "stale".into(),
            graph(),
            true,
            None,
            false,
            Some(&flow.updated_at),
        ),
        Err(BusUpdateError::VersionConflict { .. })
    ));

    let disabled = set_enabled(&config, &flow.id, false).unwrap();
    assert!(!disabled.enabled);
    assert!(list_enabled_flows(&config).unwrap().0.is_empty());

    let suggestion = FlowSuggestion {
        id: "suggestion-1".into(),
        title: "Send a daily report".into(),
        one_liner: "Send a daily report.".into(),
        rationale: "It saves time.".into(),
        trigger_hint: Some("schedule".into()),
        steps_outline: vec!["Read the report".into()],
        suggested_connections: vec![],
        suggested_slugs: vec![],
        build_prompt: "Create the report workflow".into(),
        confidence: 0.8,
        status: SuggestionStatus::New,
        created_at: flow.created_at,
        source_run_id: None,
    };
    assert_eq!(
        upsert_suggestions(&config, std::slice::from_ref(&suggestion)).unwrap(),
        1
    );
    assert_eq!(
        list_suggestions(&config, Some(SuggestionStatus::New), 10)
            .unwrap()
            .len(),
        1
    );
    assert!(set_suggestion_status(&config, &suggestion.id, SuggestionStatus::Built).unwrap());
    assert_eq!(
        list_suggestions(&config, Some(SuggestionStatus::New), 10)
            .unwrap()
            .len(),
        0
    );
}
