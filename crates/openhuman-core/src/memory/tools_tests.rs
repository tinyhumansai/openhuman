use super::*;
use crate::memory::test_fixtures::{bind_reference, config_in, stored};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tinymemory::{ItemKind, MetaFilter};

struct Ctx {
    root: PathBuf,
    thread: &'static str,
}

impl ToolRunContext for Ctx {
    fn workspace_root(&self) -> Option<&Path> {
        Some(&self.root)
    }
    fn thread_id(&self) -> Option<&str> {
        Some(self.thread)
    }
}

fn facts() -> CallFacts {
    CallFacts {
        workspace: Some("/work/space".into()),
        thread_id: Some("thread-tool-1".into()),
        agent_id: Some("orchestrator".into()),
        tool_call_id: Some("call-9".into()),
    }
}

fn text(result: &ToolResult) -> String {
    result.text()
}

#[test]
fn schema_and_permissions_follow_the_action() {
    let tool = MemoryTool::new(Arc::new(Config::default()));
    assert_eq!(tool.name(), MEMORY_TOOL_NAME);
    assert!(!tool.description().is_empty());
    let schema = tool.parameters_schema();
    assert_eq!(schema["required"][0], "action");
    assert_eq!(
        schema["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
    for read in ["recall", "fetch"] {
        let args = json!({ "action": read });
        assert_eq!(
            tool.permission_level_with_args(&args),
            PermissionLevel::ReadOnly
        );
        assert!(tool.is_concurrency_safe(&args));
    }
    for write in ["learn", "forget"] {
        let args = json!({ "action": write });
        assert_eq!(
            tool.permission_level_with_args(&args),
            PermissionLevel::Write
        );
        assert!(!tool.is_concurrency_safe(&args));
    }
    assert_eq!(
        tool.permission_level_with_args(&json!({})),
        PermissionLevel::Write
    );
}

#[tokio::test]
async fn every_action_reports_memory_off_as_a_tool_error() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    for args in [
        json!({"action": "recall", "question": "q"}),
        json!({"action": "fetch", "query": "q"}),
        json!({"action": "learn", "text": "t"}),
        json!({"action": "forget", "ids": ["a"]}),
    ] {
        let result = run_action(&config, &args, &facts()).await;
        assert!(result.is_error, "{args}");
        assert!(text(&result).contains("MEMORY_OFF"), "{}", text(&result));
    }
}

#[tokio::test]
async fn bad_arguments_and_unknown_actions_are_model_visible_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);

    let unknown = run_action(&config, &json!({"action": "explode"}), &facts()).await;
    assert!(unknown.is_error);
    assert!(text(&unknown).contains("unknown action `explode`"));

    let missing = run_action(&config, &json!({}), &facts()).await;
    assert!(missing.is_error);

    for action in ["recall", "fetch", "learn", "forget"] {
        let result = run_action(&config, &json!({ "action": action }), &facts()).await;
        assert!(result.is_error, "{action}");
        assert!(text(&result).contains("invalid arguments"), "{action}");
    }
}

#[tokio::test]
async fn learn_stamps_what_the_host_knows_and_ignores_model_meta() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);

    let args = json!({
        "action": "learn",
        "text": "The user deploys on Fridays",
        "kind": "procedure",
        "confidence": 0.6,
        // The model must not be able to forge host-owned fields.
        "meta": {"thread_id": "forged", "agent_id": "forged"},
    });
    let result = run_action(&config, &args, &facts()).await;
    assert!(!result.is_error, "{}", text(&result));
    let id = serde_json::from_str::<Value>(&text(&result)).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let items = stored(
        &engine,
        MetaFilter {
            kinds: vec![ItemKind::Learning],
            ..MetaFilter::default()
        },
    )
    .await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id.0, id);
    let meta = &items[0].meta;
    assert_eq!(meta.workspace.as_deref(), Some("/work/space"));
    assert_eq!(meta.thread_id.as_deref(), Some("thread-tool-1"));
    assert_eq!(meta.agent_id.as_deref(), Some("orchestrator"));
    let call = meta.tool_call.as_ref().expect("tool_call stamped");
    assert_eq!(call.name, MEMORY_TOOL_NAME);
    assert_eq!(call.id.as_deref(), Some("call-9"));
    assert_eq!(meta.source.kind, SourceKind::Agent);
}

#[tokio::test]
async fn recall_fetch_and_forget_round_trip_and_recall_records_citations() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let facts = CallFacts {
        thread_id: Some("thread-citations".into()),
        ..facts()
    };

    let learned = run_action(
        &config,
        &json!({"action": "learn", "text": "The user likes oolong tea"}),
        &facts,
    )
    .await;
    let id = serde_json::from_str::<Value>(&text(&learned)).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let recalled = run_action(
        &config,
        &json!({"action": "recall", "question": "oolong tea"}),
        &facts,
    )
    .await;
    assert!(!recalled.is_error, "{}", text(&recalled));
    let cites = take_turn_citations("thread-citations");
    assert!(cites.iter().any(|c| c.id == id), "{cites:?}");
    assert!(
        take_turn_citations("thread-citations").is_empty(),
        "draining empties the thread's citations"
    );

    let fetched = run_action(
        &config,
        &json!({"action": "fetch", "query": "oolong", "mode": "keyword", "limit": 5}),
        &facts,
    )
    .await;
    assert!(!fetched.is_error, "{}", text(&fetched));
    assert_eq!(
        serde_json::from_str::<Value>(&text(&fetched)).unwrap()["hits"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let forgotten = run_action(&config, &json!({"action": "forget", "ids": [id]}), &facts).await;
    assert!(!forgotten.is_error, "{}", text(&forgotten));
    assert!(engine.is_empty());
}

#[tokio::test]
async fn recall_without_a_thread_records_no_citations() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);
    let no_thread = CallFacts {
        thread_id: None,
        ..facts()
    };
    run_action(
        &config,
        &json!({"action": "learn", "text": "fact one"}),
        &no_thread,
    )
    .await;
    let result = run_action(
        &config,
        &json!({"action": "recall", "question": "fact one"}),
        &no_thread,
    )
    .await;
    assert!(!result.is_error);
}

#[tokio::test]
async fn fetch_with_an_undeclared_mode_is_unsupported() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    crate::memory::ops::apply_engine_set(
        &mut config,
        &crate::memory::types::EngineSetParams {
            engine: crate::memory::engine::CORTEXDB_ENGINE.into(),
            endpoint: Some("https://cortex.example.test".into()),
            api_key: Some("cdb-test-key".into()),
        },
    )
    .unwrap();
    let result = run_action(
        &config,
        &json!({"action": "fetch", "query": "x", "mode": "keyword"}),
        &facts(),
    )
    .await;
    assert!(result.is_error);
    assert!(text(&result).contains("UNSUPPORTED"), "{}", text(&result));
}

#[test]
fn turn_citations_are_capped_and_deduplicated() {
    let citation = |n: usize| tinymemory::Citation {
        id: tinymemory::ItemId(format!("cite-{n}")),
        kind: ItemKind::Learning,
        snippet: "s".repeat(1000),
        score: Some(0.5),
        meta: MemoryMeta::default(),
    };
    let thread = "thread-cap";
    let many: Vec<_> = (0..MAX_TURN_CITATIONS + 10).map(citation).collect();
    record_turn_citations(thread, &many);
    record_turn_citations(thread, &many[..2]);
    record_turn_citations(thread, &[]);
    let drained = take_turn_citations(thread);
    assert_eq!(drained.len(), MAX_TURN_CITATIONS);
    assert_eq!(
        drained
            .iter()
            .map(|c| c.id.clone())
            .collect::<HashSet<_>>()
            .len(),
        MAX_TURN_CITATIONS
    );
    assert!(
        drained[0].snippet.chars().count() <= crate::memory::types::TURN_CITATION_SNIPPET_CHARS
    );
    assert_eq!(drained[0].key, "learning");
}

#[tokio::test]
async fn execute_gathers_facts_from_the_run_context() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let tool = MemoryTool::new(Arc::new(config));
    let ctx = Ctx {
        root: tmp.path().join("isolated"),
        thread: "thread-ctx",
    };
    let result = tool
        .execute_with_context(
            json!({"action": "learn", "text": "ctx fact"}),
            ToolCallOptions::default(),
            Some(&ctx),
        )
        .await
        .unwrap();
    assert!(!result.is_error, "{}", text(&result));
    let items = stored(&engine, MetaFilter::default()).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].meta.thread_id.as_deref(), Some("thread-ctx"));
    assert_eq!(
        items[0].meta.workspace.as_deref(),
        Some(tmp.path().join("isolated").display().to_string().as_str())
    );

    // No context: workspace falls back to the config's action dir.
    let plain = tool
        .execute(json!({"action": "learn", "text": "plain fact"}))
        .await
        .unwrap();
    assert!(!plain.is_error);
}

#[test]
fn gather_falls_back_to_the_action_dir() {
    let config = Config {
        action_dir: PathBuf::from("/the/action/dir"),
        ..Config::default()
    };
    let gathered = CallFacts::gather(&config, None);
    assert_eq!(gathered.workspace.as_deref(), Some("/the/action/dir"));
    assert!(gathered.thread_id.is_none());
    assert!(gathered.tool_call_id.is_none());
    let meta = gathered.learn_meta();
    assert_eq!(meta.source.kind, SourceKind::Agent);
    assert_eq!(meta.tool_call.unwrap().name, MEMORY_TOOL_NAME);
}
