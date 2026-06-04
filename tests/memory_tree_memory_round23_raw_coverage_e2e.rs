//! Round 23 focused raw coverage for memory + memory_tree gaps.
//!
//! These tests stay hermetic: temp workspaces only, no real Ollama process,
//! and no networked embedding service.

use std::ffi::OsString;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Result;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use serde_json::{json, Map, Value};
use tempfile::TempDir;

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::embeddings::NoopEmbedding;
use openhuman_core::openhuman::inference::provider::traits::Provider;
use openhuman_core::openhuman::memory::{
    ExtractionMode, MemoryIngestionConfig, MemoryIngestionRequest,
};
use openhuman_core::openhuman::memory_store::{NamespaceDocumentInput, UnifiedMemory};
use openhuman_core::openhuman::memory_tree::tree_runtime::{
    all_tree_summarizer_registered_controllers, engine, rpc as tree_runtime_rpc,
    store as tree_runtime_store,
};

struct EnvVarGuard {
    key: &'static str,
    old: Option<OsString>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, value: &Path) -> Self {
        let old = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value.as_os_str());
        }
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.old {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn config_in(tmp: &TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().to_path_buf(),
        ..Config::default()
    }
}

struct ScriptedProvider {
    response: String,
}

#[async_trait]
impl Provider for ScriptedProvider {
    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> Result<String> {
        let system_prompt = system_prompt.expect("tree runtime should pass a system prompt");
        assert!(system_prompt.contains("hierarchical summarizer"));
        assert!(system_prompt.contains("under"));
        assert!(!message.trim().is_empty());
        assert!(!model.trim().is_empty());
        assert!(temperature > 0.0);
        Ok(self.response.clone())
    }
}

#[tokio::test]
async fn ingestion_parser_recovers_headers_project_preferences_and_relations() {
    let tmp = TempDir::new().expect("tempdir");
    let memory =
        UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).expect("memory store");

    let content = r#"
From: Alice Example <alice@example.com>
To: Bob Builder <bob@example.com>
CC: Clara Ops <clara@example.com>
Subject: OpenHuman round 23 memory coverage
Date: 2026-05-30

# Coverage Launch
Project name: OpenHuman
Subproject: Memory Tree Round 23
Name: Parser coverage sweep
Owner: Alice Example
Due date: 2026-06-02
Target milestone: Round 23 green coverage
Preferred embedding model for local experiments: bge-m3
Preferred extraction mode to try first: sentence mode
Alice Example owns Parser coverage sweep.
OpenHuman uses JSON-RPC.
Clara Ops prefers core-first delivery.
The board is spatially near the memory tree dashboard.
Bob Builder will review the memory tree recap.
"#;

    let result = memory
        .ingest_document(MemoryIngestionRequest {
            document: NamespaceDocumentInput {
                namespace: "round23 memory".into(),
                key: "parser-coverage".into(),
                title: "OpenHuman parser coverage".into(),
                content: content.into(),
                source_type: "gmail".into(),
                priority: "high".into(),
                tags: vec!["seed".into()],
                metadata: json!({"round": 23}),
                category: "coverage".into(),
                session_id: Some("round23-session".into()),
                document_id: None,
                taint: openhuman_core::openhuman::memory::MemoryTaint::Internal,
            },
            config: MemoryIngestionConfig {
                model_name: "round23-heuristic".into(),
                extraction_mode: ExtractionMode::Chunk,
                ..MemoryIngestionConfig::default()
            },
        })
        .await
        .expect("ingest document");

    assert_eq!(result.namespace, "round23_memory");
    assert_eq!(result.extraction_mode, "chunk");
    assert!(result.chunk_count >= 1);
    assert!(result.entity_count >= 5, "entities: {:?}", result.entities);
    assert!(
        result.relation_count >= 6,
        "relations: {:?}",
        result.relations
    );
    assert!(result.preference_count >= 1);
    assert!(result.decision_count >= 2);
    assert!(result.tags.iter().any(|tag| tag == "deadline"));
    assert!(result.tags.iter().any(|tag| tag == "decision"));
    assert!(result.tags.iter().any(|tag| tag == "seed"));
    assert!(result
        .entities
        .iter()
        .any(|entity| entity.name == "ALICE EXAMPLE"));
    assert!(result
        .relations
        .iter()
        .any(|relation| relation.subject == "ALICE EXAMPLE" && relation.predicate == "OWNS"));
    assert!(result
        .relations
        .iter()
        .any(|relation| relation.subject == "OPENHUMAN"
            && relation.predicate == "USES"
            && relation.object.contains("JSON-RPC")));
    assert!(result
        .relations
        .iter()
        .any(|relation| relation.predicate == "HAS_DEADLINE"));
    assert!(result
        .relations
        .iter()
        .any(|relation| relation.predicate == "PREFERS"));

    let graph_rows = memory
        .graph_query_namespace("round23 memory", Some("ALICE EXAMPLE"), Some("OWNS"))
        .await
        .expect("query graph");
    assert!(
        graph_rows.iter().any(|row| row
            .get("object")
            .and_then(Value::as_str)
            .map(|object| object.contains("PARSER"))
            .unwrap_or(false)),
        "graph rows: {graph_rows:?}"
    );
}

#[tokio::test]
async fn tree_runtime_engine_summarizes_preserves_buffer_and_rebuilds() {
    let tmp = TempDir::new().expect("tempdir");
    let config = config_in(&tmp);
    let provider = ScriptedProvider {
        response: "round23 summary ".repeat(32),
    };
    let namespace = "round23/tree runtime";
    let ts = Utc.with_ymd_and_hms(2026, 5, 30, 10, 15, 0).unwrap();

    tree_runtime_store::buffer_write(
        &config,
        namespace,
        "first raw memory tree entry",
        &ts,
        Some(&json!({"source": "round23"})),
    )
    .expect("buffer first entry");
    tree_runtime_store::buffer_write(
        &config,
        namespace,
        "second raw memory tree entry",
        &ts,
        None,
    )
    .expect("buffer second entry");

    let hour = engine::run_summarization(&config, &provider, "test-model", namespace, ts)
        .await
        .expect("run summarization")
        .expect("hour node");
    assert_eq!(hour.node_id, "2026/05/30/10");
    assert_eq!(hour.child_count, 0);
    assert!(tree_runtime_store::buffer_read(&config, namespace)
        .expect("buffer drained")
        .is_empty());

    for node_id in ["2026/05/30", "2026/05", "2026", "root"] {
        let node = tree_runtime_store::read_node(&config, namespace, node_id)
            .expect("read propagated node")
            .unwrap_or_else(|| panic!("missing propagated node {node_id}"));
        assert!(node.child_count >= 1);
        assert!(node.summary.contains("round23 summary") || node.summary.contains("##"));
    }

    assert!(
        engine::run_summarization(&config, &provider, "test-model", namespace, ts)
            .await
            .expect("empty run")
            .is_none()
    );

    tree_runtime_store::buffer_write(
        &config,
        namespace,
        "pending buffer entry should survive rebuild",
        &ts,
        None,
    )
    .expect("buffer pending entry");

    let status = engine::rebuild_tree(&config, &provider, "test-model", namespace)
        .await
        .expect("rebuild tree");
    assert!(status.total_nodes >= 5);
    let pending =
        tree_runtime_store::buffer_read(&config, namespace).expect("buffer after rebuild");
    assert_eq!(pending.len(), 1);
    assert!(pending[0]
        .1
        .contains("pending buffer entry should survive rebuild"));
}

#[tokio::test]
async fn tree_runtime_rpc_and_registered_handlers_cover_status_and_errors() {
    let _lock = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let config = config_in(&tmp);
    let _workspace = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", tmp.path());
    let timestamp = Utc.with_ymd_and_hms(2026, 5, 30, 11, 0, 0).unwrap();

    let ingest = tree_runtime_rpc::tree_summarizer_ingest(
        &config,
        " round23 rpc ",
        "handler-routed buffered content",
        Some(timestamp),
        Some(&json!({"handler": true})),
    )
    .await
    .expect("direct rpc ingest")
    .value;
    assert_eq!(ingest["buffered"], true);
    assert_eq!(ingest["namespace"], "round23 rpc");
    assert_eq!(ingest["has_metadata"], true);

    let status = tree_runtime_rpc::tree_summarizer_status(&config, "round23 rpc")
        .await
        .expect("status")
        .value;
    assert_eq!(status["namespace"], "round23 rpc");
    assert_eq!(status["total_nodes"], 0);

    let err = tree_runtime_rpc::tree_summarizer_query(&config, "round23 rpc", Some("root"))
        .await
        .expect_err("root not created yet");
    assert!(err.contains("node 'root' not found"));
    assert!(
        tree_runtime_rpc::tree_summarizer_ingest(&config, "../bad", "x", None, None)
            .await
            .expect_err("bad namespace")
            .contains("..")
    );

    let controllers = all_tree_summarizer_registered_controllers();
    assert_eq!(controllers.len(), 5);
    assert!(controllers
        .iter()
        .any(|controller| controller.rpc_method_name() == "openhuman.tree_summarizer_ingest"));

    let ingest_handler = controllers
        .iter()
        .find(|controller| controller.schema.function == "ingest")
        .expect("ingest controller")
        .handler;
    let mut params = Map::<String, Value>::new();
    params.insert("namespace".into(), json!("round23-handler"));
    params.insert("content".into(), json!("handler content"));
    params.insert("timestamp".into(), json!("2026-05-30T12:00:00Z"));
    params.insert("metadata".into(), json!({"via": "registered-controller"}));
    let handler_value = ingest_handler(params).await.expect("handler ingest");
    assert_eq!(handler_value["result"]["buffered"], true);
    assert!(handler_value["logs"][0]
        .as_str()
        .unwrap()
        .contains("content buffered"));

    let status_handler = controllers
        .iter()
        .find(|controller| controller.schema.function == "status")
        .expect("status controller")
        .handler;
    let missing_err = status_handler(Map::new())
        .await
        .expect_err("missing namespace should fail");
    assert!(missing_err.contains("missing required param 'namespace'"));
}
