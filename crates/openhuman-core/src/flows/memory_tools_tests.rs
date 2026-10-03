use super::*;
use crate::memory::types::ItemsListParams;
use tempfile::TempDir;
use tinymemory::conformance::ReferenceEngine;

/// A config whose workspace is bound to a fresh in-memory reference engine.
fn engine_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().to_path_buf();
    crate::memory::engine::install_test_engine(
        &config.workspace_dir,
        Arc::new(ReferenceEngine::new()),
    );
    (tmp, config)
}

/// A config with no engine bound (memory off).
fn off_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().to_path_buf();
    config.memory.engine = String::new();
    (tmp, config)
}

async fn list(config: &Config, filter: MetaFilter) -> Vec<tinymemory::Hit> {
    crate::memory::ops::items_list(
        config,
        ItemsListParams {
            filter: Some(filter),
            limit: Some(100),
            cursor: None,
        },
    )
    .await
    .unwrap()
    .items
}

// ── tags ────────────────────────────────────────────────────────

#[test]
fn flow_tag_is_distinct_per_flow() {
    assert_eq!(flow_tag("abc-123"), "flow:abc-123");
    assert_ne!(flow_tag("a"), flow_tag("b"));
}

#[test]
fn flow_meta_carries_flow_and_cross_flow_tags() {
    let meta = flow_meta("f1", &["extra".to_string()]);
    assert_eq!(meta.tags, vec!["flow:f1", FLOWS_TAG, "extra"]);
    assert_eq!(meta.source.kind, SourceKind::Agent);
    assert_eq!(meta.source.id.as_deref(), Some("flow:f1"));
}

#[test]
fn flow_key_round_trips_through_tags() {
    let meta = flow_meta("f1", &[flow_key_tag("f1", "sent:42")]);
    assert_eq!(flow_key_of(&meta), Some("sent:42"));
    assert_eq!(flow_key_of(&flow_meta("f1", &[])), None);
}

#[test]
fn learning_kind_maps_categories() {
    assert_eq!(learning_kind_for(None), LearningKind::Fact);
    assert_eq!(learning_kind_for(Some("core")), LearningKind::Fact);
    assert_eq!(
        learning_kind_for(Some("Preference")),
        LearningKind::Preference
    );
    assert_eq!(learning_kind_for(Some("daily")), LearningKind::Other);
}

// ── keyed writes / forget ───────────────────────────────────────

#[tokio::test]
async fn remember_keyed_replaces_the_previous_value() {
    let (_tmp, config) = engine_config();
    remember_keyed(&config, "f1", "k", "first", LearningKind::Fact)
        .await
        .unwrap();
    remember_keyed(&config, "f1", "k", "second", LearningKind::Fact)
        .await
        .unwrap();
    let items = list(&config, flow_filter("f1")).await;
    assert_eq!(items.len(), 1);
    assert!(items[0].text.contains("second"));
    assert_eq!(flow_key_of(&items[0].meta), Some("k"));
}

#[tokio::test]
async fn flow_filters_isolate_flows_and_cross_flow_sees_all() {
    let (_tmp, config) = engine_config();
    remember_keyed(&config, "f1", "k", "one", LearningKind::Fact)
        .await
        .unwrap();
    remember_keyed(&config, "f2", "k", "two", LearningKind::Fact)
        .await
        .unwrap();
    assert_eq!(list(&config, flow_filter("f1")).await.len(), 1);
    assert_eq!(list(&config, cross_flow_filter()).await.len(), 2);
    assert_eq!(
        forget_matching(&config, flow_filter("f1")).await.unwrap(),
        1
    );
    assert_eq!(list(&config, cross_flow_filter()).await.len(), 1);
}

#[tokio::test]
async fn forget_matching_refuses_an_empty_filter() {
    let (_tmp, config) = engine_config();
    let err = forget_matching(&config, MetaFilter::default())
        .await
        .unwrap_err();
    assert_eq!(err.code(), crate::memory::error::INVALID_REQUEST);
}

#[tokio::test]
async fn memory_off_forgets_nothing_and_refuses_writes() {
    let (_tmp, config) = off_config();
    assert_eq!(
        forget_matching(&config, flow_filter("f1")).await.unwrap(),
        0
    );
    let err = remember_keyed(&config, "f1", "k", "v", LearningKind::Fact)
        .await
        .unwrap_err();
    assert_eq!(err.code(), crate::memory::error::MEMORY_OFF);
}

// ── FlowMemoryRecallTool ────────────────────────────────────────

#[test]
fn recall_name_and_schema() {
    let tool = FlowMemoryRecallTool::new();
    assert_eq!(tool.name(), "flow_memory_recall");
    let schema = tool.parameters_schema();
    assert!(schema["properties"]["query"].is_object());
    assert!(schema["properties"]["flow_id"].is_object());
    assert!(schema["properties"]["scope"].is_object());
}

#[tokio::test]
async fn recall_rejects_unknown_scope() {
    let result = FlowMemoryRecallTool::new()
        .execute(json!({"query": "q", "flow_id": "f1", "scope": "user"}))
        .await
        .unwrap();
    assert!(result.is_error);
}

#[test]
fn render_recall_reports_no_citations() {
    assert!(render_recall("x", &[]).contains("No flow memories"));
}

// ── FlowMemoryRememberTool ──────────────────────────────────────

#[test]
fn remember_name_and_schema() {
    let tool = FlowMemoryRememberTool::new(Arc::new(SecurityPolicy::default()));
    assert_eq!(tool.name(), "flow_memory_remember");
    let schema = tool.parameters_schema();
    assert!(schema["properties"]["flow_id"].is_object());
    assert!(schema["properties"]["key"].is_object());
    assert!(schema["properties"]["content"].is_object());
    // No `namespace`/`tags` parameter: a flow can never target another
    // flow's memory.
    assert!(schema["properties"]["namespace"].is_null());
    assert!(schema["properties"]["tags"].is_null());
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
}

#[tokio::test]
async fn remember_outside_a_workflow_run_is_refused() {
    let tool = FlowMemoryRememberTool::new(Arc::new(SecurityPolicy::default()));
    let result = tool
        .execute(json!({"flow_id": "f1", "key": "k", "content": "v"}))
        .await
        .unwrap();
    assert!(result.is_error);
}
