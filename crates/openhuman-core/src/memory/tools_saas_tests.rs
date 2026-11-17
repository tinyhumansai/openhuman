use super::*;
use crate::config::schema::MemoryAgentConfig;
use crate::memory::test_fixtures::config_in;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tinymemory_api::conformance::ReferenceEngine;
use tinymemory_api::{
    EngineDescriptor, FetchPage, FetchRequest, ForgetReport, ForgetTarget, LearningKind, ListPage,
    ListRequest, MemoryEngine, MemoryMeta, Namespace, RecallAnswer, RecallRequest, StoreItem,
    StoreReceipt,
};

struct CountingReadEngine {
    inner: ReferenceEngine,
    reads: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl MemoryEngine for CountingReadEngine {
    fn descriptor(&self) -> &EngineDescriptor {
        self.inner.descriptor()
    }

    async fn health(&self) -> tinymemory_api::EngineHealth {
        self.inner.health().await
    }

    async fn recall(&self, request: RecallRequest) -> tinymemory_api::Result<RecallAnswer> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.inner.recall(request).await
    }

    async fn fetch(&self, request: FetchRequest) -> tinymemory_api::Result<FetchPage> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.inner.fetch(request).await
    }

    async fn store(&self, item: StoreItem) -> tinymemory_api::Result<StoreReceipt> {
        self.inner.store(item).await
    }

    async fn forget(&self, target: ForgetTarget) -> tinymemory_api::Result<ForgetReport> {
        self.inner.forget(target).await
    }

    async fn list(&self, request: ListRequest) -> tinymemory_api::Result<ListPage> {
        self.inner.list(request).await
    }
}

fn tool_context_for(agent_id: &str) -> tinyagents_harness::tool::ToolExecutionContext {
    let mut host = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    host.memory_identity = Some(tinymemory_tools::layout::identity::Identity::agent(
        agent_id,
    ));
    let run = host.into_tinyagents(tinyagents_harness::context::RunConfig::new("saas-memory"));
    tinyagents_harness::tool::ToolExecutionContext::from_run_context(
        &run,
        tinyagents_harness::ids::CallId::new("saas-memory-call"),
    )
}

fn isolate(test_name: &str, marker: &str) -> Option<std::process::Output> {
    if std::env::var_os(marker).is_some() {
        None
    } else {
        Some(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", test_name, "--nocapture"])
                .env(marker, "1")
                .output()
                .unwrap(),
        )
    }
}

#[test]
fn invalid_saas_roots_and_agent_overrides_reject_memory_tool_reads() {
    const MARKER: &str = "OPENHUMAN_TEST_SAAS_MEMORY_TOOL_INVALID_ROOT";
    const TEST_NAME: &str = "memory::tools::saas_tests::invalid_saas_roots_and_agent_overrides_reject_memory_tool_reads";
    if let Some(output) = isolate(TEST_NAME, MARKER) {
        assert!(
            output.status.success()
                && String::from_utf8_lossy(&output.stdout)
                    .contains("SAAS_MEMORY_TOOL_INVALID_ROOT_CHILD_PASSED"),
            "isolated SaaS tool root test failed or ran zero tests:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    crate::core::runtime::mode::lock_mode(crate::core::runtime::Mode::Saas).unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            for root in [None, Some("not a namespace!"), Some("root")] {
                let tmp = tempfile::tempdir().unwrap();
                let mut config = config_in(&tmp);
                config.memory.root = root.map(str::to_string);
                config.memory.agents.insert(
                    "scout".into(),
                    MemoryAgentConfig {
                        root: Some("team:foreign".into()),
                        ..MemoryAgentConfig::default()
                    },
                );
                let inner = ReferenceEngine::new();
                inner
                    .store(StoreItem::learning(
                        "foreign marker that must stay unread",
                        LearningKind::Fact,
                        0.9,
                        MemoryMeta {
                            namespace: "team:foreign".parse().unwrap(),
                            ..MemoryMeta::default()
                        },
                    ))
                    .await
                    .unwrap();
                let reads = Arc::new(AtomicUsize::new(0));
                crate::memory::engine::install_test_engine(
                    &config.workspace_dir,
                    Arc::new(CountingReadEngine {
                        inner,
                        reads: reads.clone(),
                    }),
                );
                let tool = MemoryTool::new(Arc::new(config.clone()));
                let context = tool_context_for("scout");
                for args in [
                    json!({"action":"recall","question":"foreign marker"}),
                    json!({"action":"fetch","query":"foreign marker"}),
                ] {
                    let result = tool
                        .execute_with_context(args, ToolCallOptions::default(), Some(&context))
                        .await
                        .unwrap();
                    assert!(result.is_error, "root {root:?}: {}", result.text());
                    assert!(
                        result
                            .text()
                            .contains("memory is unavailable: this agent has no valid memory root"),
                        "root {root:?}: {}",
                        result.text()
                    );
                }
                assert_eq!(reads.load(Ordering::SeqCst), 0, "root {root:?}");
            }
        });
    println!("SAAS_MEMORY_TOOL_INVALID_ROOT_CHILD_PASSED");
}

#[test]
fn valid_saas_root_keeps_tool_reads_inside_the_authenticated_namespace() {
    const MARKER: &str = "OPENHUMAN_TEST_SAAS_MEMORY_TOOL_VALID_ROOT";
    const TEST_NAME: &str = "memory::tools::saas_tests::valid_saas_root_keeps_tool_reads_inside_the_authenticated_namespace";
    if let Some(output) = isolate(TEST_NAME, MARKER) {
        assert!(
            output.status.success()
                && String::from_utf8_lossy(&output.stdout)
                    .contains("SAAS_MEMORY_TOOL_VALID_ROOT_CHILD_PASSED"),
            "isolated SaaS tool clamp test failed or ran zero tests:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    crate::core::runtime::mode::lock_mode(crate::core::runtime::Mode::Saas).unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let tmp = tempfile::tempdir().unwrap();
            let mut config = config_in(&tmp);
            config.memory.root = Some("team:acme".into());
            config.memory.agents.insert(
                "scout".into(),
                MemoryAgentConfig {
                    root: Some("team:foreign".into()),
                    ..MemoryAgentConfig::default()
                },
            );
            let inner = ReferenceEngine::new();
            for (namespace, text) in [
                ("team:acme", "acme boundary marker"),
                ("team:foreign", "foreign boundary marker"),
            ] {
                inner
                    .store(StoreItem::learning(
                        text,
                        LearningKind::Fact,
                        0.9,
                        MemoryMeta {
                            namespace: namespace.parse::<Namespace>().unwrap(),
                            ..MemoryMeta::default()
                        },
                    ))
                    .await
                    .unwrap();
            }
            let reads = Arc::new(AtomicUsize::new(0));
            crate::memory::engine::install_test_engine(
                &config.workspace_dir,
                Arc::new(CountingReadEngine {
                    inner,
                    reads: reads.clone(),
                }),
            );
            let tool = MemoryTool::new(Arc::new(config));
            let context = tool_context_for("scout");
            let recall = tool
                .execute_with_context(
                    json!({"action":"recall","question":"acme boundary marker"}),
                    ToolCallOptions::default(),
                    Some(&context),
                )
                .await
                .unwrap();
            assert!(!recall.is_error, "{}", recall.text());
            let evidence: Value = serde_json::from_str(&recall.text()).unwrap();
            assert!(evidence["evidence"]
                .as_str()
                .unwrap()
                .contains("acme boundary marker"));
            assert!(!evidence["evidence"]
                .as_str()
                .unwrap()
                .contains("foreign boundary marker"));

            let fetch = tool
                .execute_with_context(
                    json!({"action":"fetch","query":"boundary marker"}),
                    ToolCallOptions::default(),
                    Some(&context),
                )
                .await
                .unwrap();
            assert!(!fetch.is_error, "{}", fetch.text());
            let page: Value = serde_json::from_str(&fetch.text()).unwrap();
            assert!(page["matches"]
                .as_str()
                .unwrap()
                .contains("acme boundary marker"));
            assert!(!page["matches"]
                .as_str()
                .unwrap()
                .contains("foreign boundary marker"));
            assert_eq!(reads.load(Ordering::SeqCst), 2);
        });
    println!("SAAS_MEMORY_TOOL_VALID_ROOT_CHILD_PASSED");
}
