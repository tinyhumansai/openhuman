use super::*;

fn ambient() -> DispatchCtx {
    DispatchCtx::ambient()
}
use crate::memory::test_fixtures::{bind_reference, config_in, stored};
use crate::memory::types::LearnParams;
use std::path::{Path, PathBuf};
use tinymemory_api::{ItemKind, MetaFilter, SourceKind};

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
        ..CallFacts::of(
            &Config::default(),
            &tinymemory_tools::layout::identity::Identity::agent("orchestrator"),
        )
    }
}

/// The facts of a call made by `agent_id` (outside any team).
fn facts_of(agent_id: &str) -> CallFacts {
    CallFacts {
        thread_id: Some(format!("thread-{agent_id}")),
        ..CallFacts::of(
            &Config::default(),
            &tinymemory_tools::layout::identity::Identity::agent(agent_id),
        )
    }
}

fn text(result: &ToolResult) -> String {
    result.text()
}

fn expected_learn_meta(facts: &CallFacts) -> tinymemory_api::MemoryMeta {
    tinymemory_api::MemoryMeta {
        namespace: facts.namespace.clone(),
        workspace: facts.workspace.clone(),
        thread_id: facts.thread_id.clone(),
        agent_id: facts.agent_id.clone(),
        tool_call: Some(tinymemory_api::ToolCallRef {
            name: MEMORY_TOOL_NAME.to_string(),
            id: facts.tool_call_id.clone(),
        }),
        source: tinymemory_api::SourceRef {
            kind: SourceKind::Agent,
            id: None,
        },
        ..tinymemory_api::MemoryMeta::default()
    }
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
    let mut config = config_in(&tmp);
    config.components.memory = false;
    for args in [
        json!({"action": "recall", "question": "q"}),
        json!({"action": "fetch", "query": "q"}),
        json!({"action": "learn", "text": "t"}),
        json!({"action": "forget", "ids": ["a"]}),
    ] {
        let result = run_action(&ambient(), &config, &args, &facts()).await;
        assert!(result.is_error, "{args}");
        assert!(text(&result).contains("MEMORY_OFF"), "{}", text(&result));
    }
}

#[tokio::test]
async fn bad_arguments_and_unknown_actions_are_model_visible_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);

    let unknown = run_action(&ambient(), &config, &json!({"action": "explode"}), &facts()).await;
    assert!(unknown.is_error);
    assert!(text(&unknown).contains("unknown action"));
    assert!(text(&unknown).contains("explode"));

    let missing = run_action(&ambient(), &config, &json!({}), &facts()).await;
    assert!(missing.is_error);
    assert!(text(&missing).contains("action is required"));

    for action in ["recall", "fetch", "learn", "forget"] {
        let result = run_action(&ambient(), &config, &json!({ "action": action }), &facts()).await;
        assert!(result.is_error, "{action}");
        assert!(
            text(&result).contains("INVALID_REQUEST"),
            "{action}: {}",
            text(&result)
        );
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
    let result = run_action(&ambient(), &config, &args, &facts()).await;
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
        &ambient(),
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
        &ambient(),
        &config,
        &json!({"action": "recall", "question": "oolong tea"}),
        &facts,
    )
    .await;
    assert!(!recalled.is_error, "{}", text(&recalled));
    let cites = crate::web_chat::citations::take_turn_citations(&ambient(), "thread-citations");
    assert_eq!(cites.len(), 1, "{cites:?}");
    assert_eq!(cites[0].id, id, "citation ids stay available to the host");
    assert!(
        crate::web_chat::citations::take_turn_citations(&ambient(), "thread-citations").is_empty(),
        "draining empties the thread's citations"
    );

    let fetched = run_action(
        &ambient(),
        &config,
        &json!({"action": "fetch", "query": "oolong", "mode": "keyword", "limit": 5}),
        &facts,
    )
    .await;
    assert!(!fetched.is_error, "{}", text(&fetched));
    assert_eq!(
        serde_json::from_str::<Value>(&text(&fetched)).unwrap()["items"],
        1
    );

    let forgotten = run_action(
        &ambient(),
        &config,
        &json!({"action": "forget", "ids": [id]}),
        &facts,
    )
    .await;
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
        &ambient(),
        &config,
        &json!({"action": "learn", "text": "fact one"}),
        &no_thread,
    )
    .await;
    let result = run_action(
        &ambient(),
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
        &ambient(),
        &config,
        &json!({"action": "fetch", "query": "x", "mode": "keyword"}),
        &facts(),
    )
    .await;
    assert!(result.is_error);
    assert!(
        text(&result).contains("INVALID_REQUEST"),
        "{}",
        text(&result)
    );
    assert!(text(&result).contains("mode"), "{}", text(&result));
    assert!(text(&result).contains("hybrid"), "{}", text(&result));
}

#[tokio::test]
async fn execute_gathers_facts_from_the_run_context() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let tool = MemoryTool::new(Arc::new(config.clone()));
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
    super::super::tool_writes::drain(&config).await;
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
    super::super::tool_writes::drain(&config).await;
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
    let meta = expected_learn_meta(&gathered);
    assert!(
        meta.namespace.is_root(),
        "no agent in scope: the default root"
    );
    assert_eq!(meta.source.kind, SourceKind::Agent);
    assert_eq!(meta.tool_call.unwrap().name, MEMORY_TOOL_NAME);
}

#[tokio::test]
async fn learnings_are_shared_under_a_root_and_a_team_root_is_kept_apart() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let member = || CallFacts {
        thread_id: Some("thread-team".into()),
        ..CallFacts::of(
            &config,
            &tinymemory_tools::layout::identity::Identity::team_member("acme", "writer"),
        )
    };
    let learn = |facts: CallFacts, text: &'static str| {
        let config = config.clone();
        async move {
            let result = run_action(
                &ambient(),
                &config,
                &json!({"action": "learn", "text": text}),
                &facts,
            )
            .await;
            assert!(!result.is_error, "{}", result.text());
        }
    };
    learn(facts(), "the main agent knows the deploy day").await;
    learn(facts_of("researcher"), "the researcher prefers arxiv").await;
    learn(member(), "the acme team drafts in british english").await;

    let all = stored(&engine, MetaFilter::kinds([ItemKind::Learning])).await;
    let at = |text: &str| {
        all.iter()
            .find(|hit| hit.text.contains(text))
            .map(|hit| (hit.meta.namespace.to_string(), hit.meta.agent_id.clone()))
            .unwrap()
    };
    assert_eq!(
        at("deploy day"),
        ("root".into(), Some("orchestrator".into()))
    );
    assert_eq!(at("arxiv"), ("root".into(), Some("researcher".into())));
    assert_eq!(
        at("british english"),
        ("team:acme".into(), Some("writer".into())),
        "a team member learns into its team's root"
    );

    let fetch = |facts: CallFacts| {
        let config = config.clone();
        async move {
            run_action(
                &ambient(),
                &config,
                &json!({"action": "fetch", "query": "the", "limit": 50}),
                &facts,
            )
            .await
            .text()
        }
    };
    let researcher = fetch(facts_of("researcher")).await;
    assert!(researcher.contains("arxiv") && researcher.contains("deploy day"));
    let team = fetch(member()).await;
    assert!(team.contains("british english"), "{team}");
    assert!(
        !team.contains("deploy day"),
        "the default root's memory is out of a team's reach: {team}"
    );

    let main_item = stored(&engine, MetaFilter::default())
        .await
        .into_iter()
        .find(|hit| hit.text.contains("deploy day"))
        .unwrap();
    let forgot = run_action(
        &ambient(),
        &config,
        &json!({"action": "forget", "ids": [main_item.id.0.clone()]}),
        &member(),
    )
    .await;
    assert!(!forgot.is_error, "{}", forgot.text());
    let ack: Value = serde_json::from_str(&forgot.text()).unwrap();
    assert_eq!(ack["queued"], 1);
    assert_eq!(ack["status"], "queued; not yet removed from memory");
    let after_forget = stored(&engine, MetaFilter::default()).await;
    assert_eq!(
        after_forget.len(),
        3,
        "all identity-scoped learnings remain"
    );
    assert!(after_forget.iter().any(|hit| hit.id == main_item.id));
    assert!(after_forget.iter().any(|hit| hit.text.contains("arxiv")));
    assert!(after_forget
        .iter()
        .any(|hit| hit.text.contains("british english")));
}

/// The reference engine, recording how far each single store waited.
struct WaitRecorder {
    inner: tinymemory_api::conformance::ReferenceEngine,
    waits: std::sync::Mutex<Vec<tinymemory_api::WaitFor>>,
}

#[async_trait]
impl tinymemory_api::MemoryEngine for WaitRecorder {
    fn descriptor(&self) -> &tinymemory_api::EngineDescriptor {
        self.inner.descriptor()
    }
    async fn health(&self) -> tinymemory_api::EngineHealth {
        self.inner.health().await
    }
    async fn recall(
        &self,
        req: tinymemory_api::RecallRequest,
    ) -> tinymemory_api::Result<tinymemory_api::RecallAnswer> {
        self.inner.recall(req).await
    }
    async fn fetch(
        &self,
        req: tinymemory_api::FetchRequest,
    ) -> tinymemory_api::Result<tinymemory_api::FetchPage> {
        self.inner.fetch(req).await
    }
    async fn store(
        &self,
        item: tinymemory_api::StoreItem,
    ) -> tinymemory_api::Result<tinymemory_api::StoreReceipt> {
        self.store_with(item, tinymemory_api::WriteOptions::visible())
            .await
    }
    async fn store_with(
        &self,
        item: tinymemory_api::StoreItem,
        options: tinymemory_api::WriteOptions,
    ) -> tinymemory_api::Result<tinymemory_api::StoreReceipt> {
        self.waits.lock().unwrap().push(options.wait);
        self.inner.store(item).await
    }
    async fn forget(
        &self,
        target: tinymemory_api::ForgetTarget,
    ) -> tinymemory_api::Result<tinymemory_api::ForgetReport> {
        self.inner.forget(target).await
    }
    async fn list(
        &self,
        req: tinymemory_api::ListRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ListPage> {
        self.inner.list(req).await
    }
    async fn consolidate(
        &self,
        req: tinymemory_api::ConsolidateRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ConsolidateReceipt> {
        self.inner.consolidate(req).await
    }
}

#[tokio::test]
async fn learn_returns_on_accept_and_says_recall_may_lag() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = Arc::new(WaitRecorder {
        inner: tinymemory_api::conformance::ReferenceEngine::new(),
        waits: std::sync::Mutex::new(Vec::new()),
    });
    crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());

    let learned = run_action(
        &ambient(),
        &config,
        &json!({"action": "learn", "text": "The user drinks oolong"}),
        &facts(),
    )
    .await;

    assert!(!learned.is_error, "{}", text(&learned));
    // A turn must never wait on the engine indexing a learning.
    assert_eq!(
        *engine.waits.lock().unwrap(),
        vec![tinymemory_api::WaitFor::Accepted]
    );
    let view = serde_json::from_str::<Value>(&text(&learned)).unwrap();
    assert!(!view["id"].as_str().unwrap().is_empty(), "{view}");
    assert_eq!(view["status"], LEARN_STATUS);

    // The RPC path still reads its own write.
    crate::memory::ops::learn(
        &config,
        LearnParams {
            text: "The user drinks puerh".into(),
            kind: None,
            confidence: None,
            meta: None,
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        engine.waits.lock().unwrap().last(),
        Some(&tinymemory_api::WaitFor::Visible)
    );
}

#[tokio::test]
async fn a_failed_learn_is_reported_to_the_model() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    std::fs::write(
        config.workspace_dir.join("memory"),
        "blocks outbox directory",
    )
    .unwrap();

    let learned = run_action(
        &ambient(),
        &config,
        &json!({"action": "learn", "text": "The user drinks oolong"}),
        &facts(),
    )
    .await;

    assert!(learned.is_error, "{}", text(&learned));
    assert!(
        text(&learned).contains("queue unavailable"),
        "{}",
        text(&learned)
    );

    let credit_tmp = tempfile::tempdir().unwrap();
    let credit_config = config_in(&credit_tmp);
    crate::memory::test_fixtures::RefusingEngine::out_of_credits().bind(&credit_config);
    let credit_error = crate::memory::ops::learn(
        &credit_config,
        LearnParams {
            text: "The user drinks oolong".into(),
            kind: None,
            confidence: None,
            meta: None,
        },
        None,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        &credit_error,
        tinymemory_api::Error::InsufficientCredits(_)
    ));
    assert_eq!(credit_error.code(), "INSUFFICIENT_CREDITS");
}

/// The reference engine, recording the date hint each read carried.
struct HintRecorder {
    inner: tinymemory_api::conformance::ReferenceEngine,
    hints: std::sync::Mutex<Vec<Option<tinymemory_api::TimeHint>>>,
}

#[async_trait]
impl tinymemory_api::MemoryEngine for HintRecorder {
    fn descriptor(&self) -> &tinymemory_api::EngineDescriptor {
        self.inner.descriptor()
    }
    async fn health(&self) -> tinymemory_api::EngineHealth {
        self.inner.health().await
    }
    async fn recall(
        &self,
        req: tinymemory_api::RecallRequest,
    ) -> tinymemory_api::Result<tinymemory_api::RecallAnswer> {
        self.hints.lock().unwrap().push(req.refers_to.clone());
        self.inner.recall(req).await
    }
    async fn fetch(
        &self,
        req: tinymemory_api::FetchRequest,
    ) -> tinymemory_api::Result<tinymemory_api::FetchPage> {
        self.hints.lock().unwrap().push(req.refers_to.clone());
        self.inner.fetch(req).await
    }
    async fn store(
        &self,
        item: tinymemory_api::StoreItem,
    ) -> tinymemory_api::Result<tinymemory_api::StoreReceipt> {
        self.inner.store(item).await
    }
    async fn forget(
        &self,
        target: tinymemory_api::ForgetTarget,
    ) -> tinymemory_api::Result<tinymemory_api::ForgetReport> {
        self.inner.forget(target).await
    }
    async fn list(
        &self,
        req: tinymemory_api::ListRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ListPage> {
        self.inner.list(req).await
    }
}

#[tokio::test]
async fn refers_to_reaches_the_engine_in_the_users_time_zone() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.user_timezone = Some("Asia/Kolkata".into());
    let engine = Arc::new(HintRecorder {
        inner: tinymemory_api::conformance::ReferenceEngine::new(),
        hints: std::sync::Mutex::new(Vec::new()),
    });
    crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());

    let fetched = run_action(
        &ambient(),
        &config,
        &json!({"action": "fetch", "query": "dinner", "refers_to": {"from": "2026-10-03", "to": "2026-10-04"}}),
        &facts(),
    )
    .await;
    assert!(!fetched.is_error, "{}", text(&fetched));
    let recalled = run_action(
        &ambient(),
        &config,
        &json!({"action": "recall", "question": "dinner", "refers_to": {"from": "2026-10-03"}}),
        &facts(),
    )
    .await;
    assert!(!recalled.is_error, "{}", text(&recalled));
    let bad = run_action(
        &ambient(),
        &config,
        &json!({"action": "fetch", "query": "dinner", "refers_to": {"from": "last Saturday"}}),
        &facts(),
    )
    .await;
    assert!(bad.is_error, "a non-date is a model-visible error");

    let day = |d| chrono::NaiveDate::from_ymd_opt(2026, 10, d).unwrap();
    let hints = engine.hints.lock().unwrap().clone();
    assert_eq!(
        hints,
        vec![
            Some(
                tinymemory_api::TimeHint::new(day(3), day(4), Some("Asia/Kolkata".into())).unwrap()
            ),
            Some(
                tinymemory_api::TimeHint::new(day(3), day(3), Some("Asia/Kolkata".into())).unwrap()
            ),
        ]
    );
}

#[tokio::test]
async fn learning_is_durably_queued_even_when_the_backend_refuses_writes() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    crate::memory::test_fixtures::RefusingEngine::out_of_credits().bind(&config);
    let tool = MemoryTool::new(Arc::new(config.clone()));
    let result = tool
        .execute(json!({"action":"learn","text":"Prefers tea"}))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.text());
    let value: Value = serde_json::from_str(&result.text()).unwrap();
    assert_eq!(value["status"], "queued; not yet saved to memory");
    assert_eq!(super::super::tool_writes::drain(&config).await, 0);
}

#[path = "tools_bound_dispatch_tests.rs"]
mod bound_dispatch_tests;
#[path = "tools_deferred_tests.rs"]
mod deferred_tests;
