use super::*;

#[test]
fn all_controller_schemas_lists_every_function() {
    let schemas = all_controller_schemas();
    let names: Vec<&'static str> = schemas.iter().map(|s| s.function).collect();
    assert_eq!(schemas.len(), 6);
    assert!(names.contains(&"report"));
    assert!(names.contains(&"cache_report"));
    assert!(names.contains(&"get_dashboard"));
    assert!(names.contains(&"get_daily_history"));
    assert!(names.contains(&"get_summary"));
    assert!(names.contains(&"get_usage_log"));
    for schema in &schemas {
        assert_eq!(schema.namespace, "cost");
    }
}

#[test]
fn all_registered_controllers_has_handlers_matching_schemas() {
    let registered = all_registered_controllers();
    assert_eq!(registered.len(), 6);
    let schema_fns: Vec<&'static str> = registered.iter().map(|r| r.schema.function).collect();
    assert!(schema_fns.contains(&"get_dashboard"));
    assert!(schema_fns.contains(&"get_daily_history"));
    assert!(schema_fns.contains(&"get_summary"));
    assert!(schema_fns.contains(&"get_usage_log"));
}

#[test]
fn schema_for_dashboard_has_no_inputs_and_one_output() {
    let s = schema_for("cost_get_dashboard");
    assert_eq!(s.function, "get_dashboard");
    assert!(s.inputs.is_empty());
    assert_eq!(s.outputs.len(), 1);
    assert_eq!(s.outputs[0].name, "dashboard");
}

#[test]
fn schema_for_daily_history_has_optional_days_input() {
    let s = schema_for("cost_get_daily_history");
    assert_eq!(s.function, "get_daily_history");
    assert_eq!(s.inputs.len(), 1);
    assert_eq!(s.inputs[0].name, "days");
    assert!(!s.inputs[0].required);
}

#[test]
fn schema_for_summary_returns_summary_output() {
    let s = schema_for("cost_get_summary");
    assert_eq!(s.function, "get_summary");
    assert_eq!(s.outputs[0].name, "summary");
}

#[test]
fn schema_for_usage_log_has_days_and_limit_inputs() {
    let s = schema_for("cost_get_usage_log");
    assert_eq!(s.function, "get_usage_log");
    assert_eq!(s.inputs.len(), 2);
    assert_eq!(s.inputs[0].name, "days");
    assert_eq!(s.inputs[1].name, "limit");
    assert_eq!(s.outputs[0].name, "usage_log");
}

#[test]
fn new_correlation_id_returns_eight_hex_chars() {
    let cid = new_correlation_id();
    assert_eq!(cid.len(), 8);
    assert!(cid.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn new_correlation_id_is_unique_across_calls() {
    let a = new_correlation_id();
    let b = new_correlation_id();
    // Collision probability for 8 hex chars (32 bits) per call is
    // ~1/4B — virtually zero for a unit test.
    assert_ne!(a, b);
}

#[test]
fn report_params_parse_camel_case_and_reject_unknown_keys() {
    let mut params = Map::new();
    params.insert("days".into(), serde_json::json!(7));
    params.insert("groupBy".into(), serde_json::json!(["day", "agent"]));
    params.insert("filter".into(), serde_json::json!({ "thread_id": "t1" }));
    let p: ReportParams = parse_report_params(params).unwrap();
    assert_eq!(p.days, Some(7));
    assert_eq!(
        p.group_by,
        vec![
            super::super::report::GroupKey::Day,
            super::super::report::GroupKey::Agent
        ]
    );
    assert_eq!(p.filter.thread_id.as_deref(), Some("t1"));

    // The wire contract is camelCase; the snake_case spelling the schema never
    // declared is rejected like any other unknown key.
    let mut snake = Map::new();
    snake.insert("group_by".into(), serde_json::json!(["model"]));
    assert!(parse_report_params::<ReportParams>(snake).is_err());

    let mut unknown = Map::new();
    unknown.insert("unexpected".into(), serde_json::json!(1));
    assert!(parse_report_params::<ReportParams>(unknown).is_err());

    let mut bad = Map::new();
    bad.insert("groupBy".into(), serde_json::json!(["colour"]));
    assert!(parse_report_params::<ReportParams>(bad).is_err());
    assert_eq!(
        parse_report_params::<ReportParams>(Map::new())
            .unwrap()
            .days,
        None
    );
}

#[test]
fn each_report_takes_only_its_own_params() {
    let mut limit = Map::new();
    limit.insert("limit".into(), serde_json::json!(5));
    assert!(parse_report_params::<ReportParams>(limit.clone()).is_err());
    let p: CacheReportParams = parse_report_params(limit).unwrap();
    assert_eq!(p.limit, Some(5));

    let mut group = Map::new();
    group.insert("groupBy".into(), serde_json::json!(["day"]));
    assert!(parse_report_params::<CacheReportParams>(group).is_err());
}

#[test]
fn a_misspelt_filter_key_is_refused() {
    let mut params = Map::new();
    params.insert("filter".into(), serde_json::json!({ "threadId": "t1" }));
    assert!(parse_report_params::<ReportParams>(params.clone()).is_err());
    assert!(parse_report_params::<CacheReportParams>(params).is_err());
}

#[test]
fn report_schemas_declare_their_inputs() {
    let report = schema_for("cost_report");
    let names: Vec<_> = report.inputs.iter().map(|f| f.name).collect();
    assert_eq!(names, vec!["days", "groupBy", "filter"]);
    assert!(report.inputs.iter().all(|f| !f.required));
    let cache = schema_for("cost_cache_report");
    assert_eq!(cache.function, "cache_report");
    assert!(cache.inputs.iter().any(|f| f.name == "limit"));
}
