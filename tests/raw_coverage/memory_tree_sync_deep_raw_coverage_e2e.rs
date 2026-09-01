//! Deep raw coverage for memory_tree + memory_sync round 18.
//!
//! Hermetic by construction: temp workspaces, no real provider APIs, and the
//! tree-summarizer CLI is driven through the local test binary.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex, OnceLock,
};

use anyhow::Result;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use serde_json::json;
use tempfile::TempDir;

use openhuman_core::openhuman::config::{Config, SchedulerGateMode};
use openhuman_core::openhuman::memory::tree::tree::rpc::{
    get_chunk_rpc, ingest_rpc, list_chunks_rpc, set_enabled_rpc, GetChunkRequest, IngestRequest,
    ListChunksRequest, SetEnabledRequest,
};
use tinymemory_core::chat::{ChatPrompt, ChatProvider};
use tinymemory_core::store::chunks::store::{set_chunk_embedding, upsert_chunks, with_connection};
use tinymemory_core::store::chunks::types::{chunk_id, Chunk, Metadata, SourceKind, SourceRef};
use tinymemory_core::store::trees::types::{SummaryNode, Tree, TreeKind};
use tinymemory_core::tree::score::embed::EMBEDDING_DIM;
use tinymemory_core::tree::score::extract::{
    EntityExtractor, EntityKind, ExtractedEntities, LlmEntityExtractor, LlmExtractorConfig,
};
use tinymemory_core::tree::score::resolver::{canonicalise, CanonicalEntity};
use tinymemory_core::tree::score::store::{index_entity, lookup_entity};
use tinymemory_core::tree::tree::set_summary_embedding;
use tinymemory_core::tree::tree::store as tree_store;
use tinymemory_core::tree::tree::TreeStatus;

struct EnvVarGuard {
    key: &'static str,
    old: Option<OsString>,
}

impl EnvVarGuard {
    fn set_path(key: &'static str, value: impl AsRef<Path>) -> Self {
        let old = std::env::var_os(key);
        unsafe { std::env::set_var(key, value.as_ref()) };
        Self { key, old }
    }

    fn set_str(key: &'static str, value: &str) -> Self {
        let old = std::env::var_os(key);
        unsafe { std::env::set_var(key, value) };
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

static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn test_config(tmp: &TempDir) -> Config {
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    cfg
}

fn cli_workspace(tmp: &TempDir) -> PathBuf {
    let workspace = tmp.path().join("cli-workspace");
    std::fs::create_dir_all(&workspace).expect("create cli workspace");
    workspace
}

fn run_core_cli(workspace: &Path, args: &[&str]) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_openhuman-core");
    Command::new(bin)
        .args(args)
        .env("OPENHUMAN_WORKSPACE", workspace)
        .env("OPENHUMAN_TRIGGER_TRIAGE_DISABLED", "1")
        .env("RUST_LOG", "warn")
        .output()
        .expect("run openhuman-core")
}

fn assert_cli_ok(workspace: &Path, args: &[&str]) -> String {
    let output = run_core_cli(workspace, args);
    assert!(
        output.status.success(),
        "CLI failed for {args:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

fn assert_cli_err(workspace: &Path, args: &[&str], expected: &str) {
    let output = run_core_cli(workspace, args);
    assert!(
        !output.status.success(),
        "CLI unexpectedly succeeded for {args:?}: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(expected),
        "stderr did not contain {expected:?}\nactual:\n{stderr}"
    );
}

fn sample_chunk(cfg: &Config, source_id: &str, seq: u32, text: &str, timestamp_ms: i64) -> Chunk {
    let ts = Utc.timestamp_millis_opt(timestamp_ms).unwrap();
    let chunk = Chunk {
        id: chunk_id(SourceKind::Chat, source_id, seq, text),
        content: text.to_string(),
        metadata: Metadata {
            source_kind: SourceKind::Chat,
            source_id: source_id.to_string(),
            owner: "round18-user".into(),
            timestamp: ts,
            time_range: (ts, ts),
            tags: vec!["round18".into()],
            source_ref: Some(SourceRef::new(format!("slack://{source_id}/{seq}"))),
            path_scope: None,
        },
        token_count: 32,
        seq_in_source: seq,
        created_at: ts,
        partial_message: false,
    };
    upsert_chunks(cfg, std::slice::from_ref(&chunk)).expect("upsert chunk");
    chunk
}

fn seed_topic_summary(
    cfg: &Config,
    entity_id: &str,
    summary_id: &str,
    score: f32,
    ts_ms: i64,
) -> SummaryNode {
    let ts = Utc.timestamp_millis_opt(ts_ms).unwrap();
    let tree = Tree {
        id: format!("tree:{summary_id}"),
        kind: TreeKind::Topic,
        scope: entity_id.to_string(),
        ask: None,
        root_id: Some(summary_id.to_string()),
        max_level: 2,
        status: TreeStatus::Active,
        created_at: ts,
        last_sealed_at: Some(ts),
    };
    tree_store::insert_tree(cfg, &tree).expect("insert topic tree");

    let node = SummaryNode {
        id: summary_id.to_string(),
        tree_id: tree.id.clone(),
        tree_kind: TreeKind::Topic,
        level: 2,
        parent_id: None,
        child_ids: vec!["child-a".into(), "child-b".into()],
        content: "Phoenix topic summary with rollout decisions and owner notes.".into(),
        token_count: 64,
        entities: vec![entity_id.to_string()],
        topics: vec!["rollout".into()],
        time_range_start: ts,
        time_range_end: ts,
        score,
        sealed_at: ts,
        deleted: false,
        embedding: None,
        doc_id: None,
        version_ms: None,
    };

    with_connection(cfg, |conn| {
        conn.execute(
            "INSERT INTO mem_tree_summaries (
                id, tree_id, tree_kind, level, parent_id,
                child_ids_json, content, token_count,
                entities_json, topics_json,
                time_range_start_ms, time_range_end_ms,
                score, sealed_at_ms, deleted, embedding,
                content_path, content_sha256
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, NULL, NULL, NULL)",
            rusqlite::params![
                node.id,
                node.tree_id,
                node.tree_kind.as_str(),
                node.level,
                node.parent_id,
                serde_json::to_string(&node.child_ids).unwrap(),
                node.content,
                node.token_count,
                serde_json::to_string(&node.entities).unwrap(),
                serde_json::to_string(&node.topics).unwrap(),
                node.time_range_start.timestamp_millis(),
                node.time_range_end.timestamp_millis(),
                node.score,
                node.sealed_at.timestamp_millis(),
                node.deleted as i64,
            ],
        )?;
        Ok(())
    })
    .expect("insert summary row");
    node
}

fn one_hot(index: usize) -> Vec<f32> {
    let mut v = vec![0.0; EMBEDDING_DIM];
    v[index] = 1.0;
    v
}

struct ScriptedChatProvider {
    responses: Vec<Result<String, String>>,
    calls: AtomicUsize,
}

impl ScriptedChatProvider {
    fn new(responses: impl IntoIterator<Item = Result<String, String>>) -> Self {
        Self {
            responses: responses.into_iter().collect(),
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ChatProvider for ScriptedChatProvider {
    fn name(&self) -> &str {
        "round18:scripted"
    }

    async fn chat_for_json(&self, prompt: &ChatPrompt) -> Result<String> {
        assert_eq!(prompt.kind, "memory_tree::extract");
        assert!(prompt.system.contains("Return JSON only"));
        assert!(prompt.user.contains("Return JSON only."));
        let idx = self.calls.fetch_add(1, Ordering::SeqCst);
        match self.responses.get(idx).cloned().unwrap_or_else(|| {
            Ok(
                r#"{"entities":[],"topics":[],"importance":0.0,"importance_reason":"empty"}"#
                    .into(),
            )
        }) {
            Ok(value) => Ok(value),
            Err(msg) => anyhow::bail!(msg),
        }
    }
}

#[test]
fn tree_summarizer_cli_covers_help_errors_file_ingest_query_and_status() {
    let tmp = TempDir::new().expect("tempdir");
    let workspace = cli_workspace(&tmp);

    let help = assert_cli_ok(&workspace, &["tree-summarizer", "--help"]);
    assert!(help.contains("tree-summarizer"));
    assert!(
        assert_cli_ok(&workspace, &["tree-summarizer", "ingest", "--help"])
            .contains("Either --content or --file is required")
    );
    assert_cli_ok(&workspace, &["tree-summarizer", "run", "--help"]);
    assert_cli_ok(&workspace, &["tree-summarizer", "query", "--help"]);
    assert_cli_ok(&workspace, &["tree-summarizer", "status", "--help"]);
    assert_cli_ok(&workspace, &["tree-summarizer", "rebuild", "--help"]);

    assert_cli_err(
        &workspace,
        &["tree-summarizer", "nonesuch"],
        "unknown tree-summarizer subcommand",
    );
    assert_cli_err(
        &workspace,
        &["tree-summarizer", "ingest", "round18-ns"],
        "either --content or --file is required",
    );
    assert_cli_err(
        &workspace,
        &[
            "tree-summarizer",
            "ingest",
            "round18-ns",
            "--file",
            "missing.md",
        ],
        "failed to read",
    );

    let empty = tmp.path().join("empty.txt");
    std::fs::write(&empty, "  \n").expect("write empty input");
    assert_cli_err(
        &workspace,
        &[
            "tree-summarizer",
            "ingest",
            "round18-ns",
            "--file",
            empty.to_str().unwrap(),
        ],
        "content is empty",
    );

    let input = tmp.path().join("notes.txt");
    std::fs::write(&input, "Alice wrote the Phoenix rollout notes.").expect("write input");
    let ingested = assert_cli_ok(
        &workspace,
        &[
            "tree-summarizer",
            "ingest",
            "round18-ns",
            "--file",
            input.to_str().unwrap(),
            "-v",
        ],
    );
    assert!(ingested.contains("\"buffered\": true"));

    let ingested_content = assert_cli_ok(
        &workspace,
        &[
            "tree-summarizer",
            "ingest",
            "round18-ns",
            "--content",
            "Bob added deployment follow-up.",
        ],
    );
    assert!(ingested_content.contains("\"namespace\": \"round18-ns\""));

    let status = assert_cli_ok(&workspace, &["tree-summarizer", "status", "round18-ns"]);
    assert!(status.contains("\"namespace\": \"round18-ns\""));
    assert!(status.contains("\"total_nodes\""));

    assert_cli_err(
        &workspace,
        &["tree-summarizer", "query", "round18-ns"],
        "node 'root' not found",
    );

    assert_cli_err(
        &workspace,
        &[
            "tree-summarizer",
            "query",
            "round18-ns",
            "--node-id",
            "missing-node",
        ],
        "invalid node_id 'missing-node'",
    );
}

#[tokio::test]
async fn llm_extractor_recovers_spans_topics_strict_filters_and_retry_paths() {
    let text = "Alice met Alice at the SF office about OAuth and PR #42.";
    let provider = Arc::new(ScriptedChatProvider::new([
        Ok(r#"{"entities":[{"kind":"person","text":"Alice"},{"kind":"person","text":"Alice"},{"kind":"location","text":"SF office"},{"kind":"technology","text":"OAuth"},{"kind":"artifact","text":"PR #42"},{"kind":"dragon","text":"hallucinated"}],"topics":[" auth flow ",""],"importance":1.8,"importance_reason":"Key migration decision"}"#.to_string()),
    ]));
    let extractor = LlmEntityExtractor::new(
        LlmExtractorConfig {
            emit_topics: true,
            output_language: Some("Spanish".into()),
            ..LlmExtractorConfig::default()
        },
        provider.clone(),
    );
    let extracted = extractor.extract(text).await.expect("extract");
    assert_eq!(provider.calls(), 1);
    assert_eq!(extracted.entities.len(), 5);
    assert_eq!(extracted.entities[0].span_start, 0);
    assert_eq!(extracted.entities[1].span_start, 10);
    assert_eq!(extracted.topics.len(), 1);
    assert_eq!(extracted.llm_importance, Some(1.0));
    assert_eq!(
        extracted.llm_importance_reason.as_deref(),
        Some("Key migration decision")
    );

    let canonical = canonicalise(&extracted);
    assert!(canonical
        .iter()
        .any(|entity| entity.canonical_id == "topic:auth flow"));

    let strict_provider = Arc::new(ScriptedChatProvider::new([Ok(
        r#"{"entities":[{"kind":"dragon","text":"Alice"},{"kind":"person","text":"Alice"}],"importance":0.4,"importance_reason":"ok"}"#.to_string(),
    )]));
    let strict = LlmEntityExtractor::new(
        LlmExtractorConfig {
            allowed_kinds: vec![EntityKind::Person],
            strict_kinds: true,
            ..LlmExtractorConfig::default()
        },
        strict_provider,
    );
    let strict_out = strict.extract(text).await.expect("strict extract");
    assert_eq!(strict_out.entities.len(), 1);
    assert_eq!(strict_out.entities[0].kind, EntityKind::Person);

    let retry_provider = Arc::new(ScriptedChatProvider::new([
        Err("transport down".to_string()),
        Ok(r#"{"entities":[{"kind":"person","text":"Alice"}],"importance":0.5,"importance_reason":"retried"}"#.to_string()),
    ]));
    let retrying = LlmEntityExtractor::new(LlmExtractorConfig::default(), retry_provider.clone());
    let retry_out = retrying.extract(text).await.expect("retry extract");
    assert_eq!(retry_provider.calls(), 2);
    assert_eq!(retry_out.entities.len(), 1);

    let truncated_provider = Arc::new(ScriptedChatProvider::new([
        Ok(r#"{"entities":[{"kind":"person","text":"Alice"}]"#.to_string()),
        Ok("not-json".to_string()),
    ]));
    let truncated = LlmEntityExtractor::new(LlmExtractorConfig::default(), truncated_provider);
    let empty_after_bad_json = truncated.extract(text).await.expect("bad json fallback");
    assert!(empty_after_bad_json.entities.is_empty());
}

/// The chunk-reading RPCs, the enable switch, and the ingest error surface.
///
/// **Status and backfill are deliberately not here any more.** They read
/// through the memory contract now, and an integration test cannot answer
/// that: `binding::install_diagnostics_for_test` is a `pub(crate)` test seam
/// this crate cannot reach, and with no driver bound, resolving one either
/// refuses — no module policy is published in a test process — or, where an
/// artifact is on the path, reads the module's own store rather than the rows
/// staged here. Writing rows and calling the handler proves nothing either way.
///
/// The coverage moved rather than being dropped, and is better placed:
///
/// - the precedence rule — `paused` > `error` > `degraded` > `syncing` >
///   `running` > `idle` — in
///   `rpc::tests::derive_pipeline_status_precedence_matches_spec`, against the
///   pure function rather than a store coaxed into each state;
/// - the handlers reading the driver's numbers, in
///   `pipeline_status_renders_the_drivers_chunk_aggregates`,
///   `pipeline_status_reflects_paused_when_scheduler_off` and
///   `backfill_status_reports_the_drivers_pending_count`, each with a
///   diagnostics driver bound;
/// - and what a real store *is* — that an ingest raises the chunk count, that
///   a deferred job stays ready without becoming eligible — in the driver's own
///   conformance suite, where a real store exists.
#[tokio::test]
async fn memory_tree_rpc_chunk_reads_set_enabled_and_ingest_errors() {
    let _lock = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvVarGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _triage = EnvVarGuard::set_str("OPENHUMAN_TRIGGER_TRIAGE_DISABLED", "1");
    let mut cfg = test_config(&tmp);
    // `list_chunks_rpc` below reads through the bound memory driver, which
    // under the `modules` gate is the loaded tinymemory artifact and resolves
    // its config from the process-wide boot policy. Publish it from THIS
    // test's config: the policy is first-call-wins and the module captures
    // its workspace at load, so the chunk seeded in-process and the rows the
    // module lists must name one store. The only driver-routed case in this
    // aggregated module, so nothing contends for the slot.
    #[cfg(feature = "modules")]
    openhuman_core::openhuman::modules::memory::set_modules_policy(Arc::new(cfg.clone()));

    let chunk = sample_chunk(
        &cfg,
        "chat:#status",
        1,
        "A chunk the chunk-reading RPCs below can find.",
        1_700_000_000_000,
    );

    // The gate is host state rather than the driver's: `set_enabled` reports
    // whether it changed anything, and which mode it landed in.
    cfg.scheduler_gate.mode = SchedulerGateMode::Off;
    let no_op = set_enabled_rpc(&mut cfg, SetEnabledRequest { enabled: false })
        .await
        .expect("set disabled no-op")
        .value;
    assert!(!no_op.changed);
    let changed = set_enabled_rpc(&mut cfg, SetEnabledRequest { enabled: true })
        .await
        .expect("set enabled")
        .value;
    assert!(changed.changed);
    assert_eq!(changed.mode, "auto");

    // One decoy per filter the request sets, so the assertion below fails if any
    // single one is ignored. With only the matching chunk in the store, a
    // handler that dropped every filter would still return exactly one row and
    // the test would pass.
    let wrong_source = sample_chunk(
        &cfg,
        "chat:#other",
        1,
        "right kind, right owner, wrong source id",
        1_700_000_001_000,
    );
    let wrong_time = sample_chunk(
        &cfg,
        "chat:#status",
        2,
        "right source, outside the window",
        1_900_000_000_000,
    );
    let wrong_owner = {
        let ts = Utc.timestamp_millis_opt(1_700_000_002_000).unwrap();
        let text = "right source and window, wrong owner";
        let chunk = Chunk {
            id: chunk_id(SourceKind::Chat, "chat:#status", 3, text),
            content: text.to_string(),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: "chat:#status".into(),
                owner: "someone-else".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec!["round18".into()],
                source_ref: None,
                path_scope: None,
            },
            token_count: 32,
            seq_in_source: 3,
            created_at: ts,
            partial_message: false,
        };
        upsert_chunks(&cfg, std::slice::from_ref(&chunk)).expect("upsert decoy");
        chunk
    };

    let wrong_kind = {
        let ts = Utc.timestamp_millis_opt(1_700_000_003_000).unwrap();
        let text = "right source, owner and window, wrong source kind";
        let chunk = Chunk {
            id: chunk_id(SourceKind::Email, "chat:#status", 4, text),
            content: text.to_string(),
            metadata: Metadata {
                source_kind: SourceKind::Email,
                source_id: "chat:#status".into(),
                owner: "round18-user".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec!["round18".into()],
                source_ref: None,
                path_scope: None,
            },
            token_count: 32,
            seq_in_source: 4,
            created_at: ts,
            partial_message: false,
        };
        upsert_chunks(&cfg, std::slice::from_ref(&chunk)).expect("upsert decoy");
        chunk
    };

    let listed = list_chunks_rpc(
        &cfg,
        ListChunksRequest {
            source_kind: Some("chat".into()),
            source_id: Some("chat:#status".into()),
            owner: Some("round18-user".into()),
            since_ms: Some(1_600_000_000_000),
            until_ms: Some(1_800_000_000_000),
            limit: Some(5),
        },
    )
    .await
    .expect("list chunks")
    .value
    .chunks;
    let listed_ids: Vec<&str> = listed.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        listed_ids,
        vec![chunk.id.as_str()],
        "every filter must discriminate: {} (source id), {} (window), {} (owner), {} (source kind) are all in the store",
        wrong_source.id,
        wrong_time.id,
        wrong_owner.id,
        wrong_kind.id
    );
    let fetched = get_chunk_rpc(
        &cfg,
        GetChunkRequest {
            id: chunk.id.clone(),
        },
    )
    .await
    .expect("get chunk")
    .value
    .chunk
    .expect("chunk exists");
    assert_eq!(fetched.id, chunk.id);
    assert!(get_chunk_rpc(
        &cfg,
        GetChunkRequest {
            id: "missing".into()
        }
    )
    .await
    .expect("missing chunk")
    .value
    .chunk
    .is_none());
    assert!(list_chunks_rpc(
        &cfg,
        ListChunksRequest {
            source_kind: Some("unknown".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap_err()
    .contains("unknown source kind"));

    let bad_chat = ingest_rpc(
        &cfg,
        IngestRequest {
            source_kind: SourceKind::Chat,
            source_id: "bad-chat".into(),
            owner: "owner".into(),
            tags: vec![],
            payload: json!({"not": "a chat batch"}),
        },
    )
    .await
    .unwrap_err();
    assert!(bad_chat.contains("invalid chat payload"));
    let bad_email = ingest_rpc(
        &cfg,
        IngestRequest {
            source_kind: SourceKind::Email,
            source_id: "bad-email".into(),
            owner: "owner".into(),
            tags: vec![],
            payload: json!({"not": "an email thread"}),
        },
    )
    .await
    .unwrap_err();
    assert!(bad_email.contains("invalid email payload"));

    let _empty_extracted = ExtractedEntities::default();
}
