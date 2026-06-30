//! Focused raw integration coverage for memory-tree and memory-sync modules.
//!
//! This suite is intentionally hermetic: every test uses a temp workspace and
//! any provider behavior is supplied by small in-process stubs. Run with
//! `--test-threads=1` because config/env and a few registries are global.

use std::ffi::OsString;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use serde_json::json;
use tempfile::TempDir;

use openhuman_core::core::event_bus::{DomainEvent, EventHandler};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::inference::provider::traits::{ChatMessage, Provider};
use openhuman_core::openhuman::memory_store::chunks::store::upsert_chunks;
use openhuman_core::openhuman::memory_store::chunks::types::{
    approx_token_count, chunk_id, Chunk, Metadata, SourceKind as ChunkSourceKind, SourceRef,
};
use openhuman_core::openhuman::memory_store::content;
use openhuman_core::openhuman::memory_store::trees::types::TreeKind;
use openhuman_core::openhuman::memory_store::trees::types::INPUT_TOKEN_BUDGET;
use openhuman_core::openhuman::memory_sync::composio::bus::{
    ComposioConfigChangedSubscriber, ComposioTriggerSubscriber,
};
use openhuman_core::openhuman::memory_sync::composio::providers::sync_state::{
    extract_item_id, DailyBudget, SyncState,
};
use openhuman_core::openhuman::memory_sync::composio::providers::{
    agent_ready_toolkits, capability_matrix, catalog_for_toolkit, classify_unknown, find_curated,
    is_action_visible_with_pref, toolkit_from_slug, toolkit_has_scope, ComposioProvider,
    CuratedTool, ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason, TaskFetchFilter,
    ToolScope, UserScopePref,
};
use openhuman_core::openhuman::memory_tree::score::extract::{EntityKind, ExtractedEntities};
use openhuman_core::openhuman::memory_tree::score::resolver::canonicalise;
use openhuman_core::openhuman::memory_tree::tree::bucket_seal::append_leaf;
use openhuman_core::openhuman::memory_tree::tree::{
    append_leaf_deferred, get_or_create_tree, store as tree_store, LabelStrategy, LeafRef,
};
use openhuman_core::openhuman::memory_tree::tree_runtime::{
    engine, rpc as tree_runtime_rpc, store as runtime_store,
};

struct EnvVarGuard {
    key: &'static str,
    old: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<Path>) -> Self {
        let old = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value.as_ref());
        }
        Self { key, old }
    }

    fn set_str(key: &'static str, value: &str) -> Self {
        let old = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
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
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    cfg
}

fn staged_chunk(cfg: &Config, source_id: &str, seq: u32, tokens: u32) -> Chunk {
    let ts = Utc
        .timestamp_millis_opt(1_700_000_000_000 + seq as i64)
        .unwrap();
    let content = format!("raw coverage chunk {source_id} {seq}");
    let chunk = Chunk {
        id: chunk_id(ChunkSourceKind::Chat, source_id, seq, &content),
        content,
        metadata: Metadata {
            source_kind: ChunkSourceKind::Chat,
            source_id: source_id.to_string(),
            owner: "coverage-user".into(),
            timestamp: ts,
            time_range: (ts, ts),
            tags: vec!["coverage".into(), "sync".into()],
            source_ref: Some(SourceRef::new(format!("chat://{source_id}/{seq}"))),
            path_scope: None,
        },
        token_count: tokens,
        seq_in_source: seq,
        created_at: ts,
        partial_message: false,
    };
    upsert_chunks(cfg, std::slice::from_ref(&chunk)).expect("upsert chunk");
    let content_root = cfg.memory_tree_content_root();
    std::fs::create_dir_all(&content_root).expect("content root");
    let staged = content::stage_chunks(&content_root, std::slice::from_ref(&chunk))
        .expect("stage chunk body");
    openhuman_core::openhuman::memory_store::chunks::store::with_connection(cfg, |conn| {
        for staged_chunk in &staged {
            conn.execute(
                "UPDATE mem_tree_chunks
                    SET content_path = ?1, content_sha256 = ?2
                  WHERE id = ?3",
                rusqlite::params![
                    staged_chunk.content_path,
                    staged_chunk.content_sha256,
                    staged_chunk.chunk.id
                ],
            )?;
        }
        Ok(())
    })
    .expect("persist staged chunk pointers");
    chunk
}

struct ScriptedProvider {
    responses: Mutex<Vec<String>>,
}

impl ScriptedProvider {
    fn new(responses: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut items: Vec<String> = responses.into_iter().map(Into::into).collect();
        items.reverse();
        Self {
            responses: Mutex::new(items),
        }
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let _ = (system_prompt, message, model, temperature);
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop()
            .unwrap_or_else(|| "fallback scripted summary".to_string()))
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let _ = (messages, model, temperature);
        self.chat_with_system(None, "", "", 0.0).await
    }
}

#[tokio::test]
async fn tree_runtime_engine_rpc_and_walk_cover_success_and_edge_paths() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg = config_in(&tmp);
    let ns = "round14-team";

    let first_ts = Utc.with_ymd_and_hms(2026, 5, 29, 10, 15, 0).unwrap();
    let second_ts = Utc.with_ymd_and_hms(2026, 5, 29, 11, 45, 0).unwrap();
    tree_runtime_rpc::tree_summarizer_ingest(
        &cfg,
        ns,
        "deployment notes mention Alice and the launch room",
        Some(first_ts),
        Some(&json!({"source": "round14"})),
    )
    .await
    .expect("ingest first");
    tree_runtime_rpc::tree_summarizer_ingest(
        &cfg,
        ns,
        "follow-up notes mention Bob and post-launch cleanup",
        Some(second_ts),
        None,
    )
    .await
    .expect("ingest second");

    let provider = ScriptedProvider::new([
        "hour 10 summary about Alice",
        "hour 11 summary about Bob",
        "rebuilt hour 10",
        "rebuilt hour 11",
    ]);
    let last = engine::run_summarization(&cfg, &provider, "test-model", ns, Utc::now())
        .await
        .expect("run summarization")
        .expect("last hour node");
    assert_eq!(last.node_id, "2026/05/29/11");
    assert!(runtime_store::buffer_read(&cfg, ns)
        .expect("buffer read after drain")
        .is_empty());

    let status = tree_runtime_rpc::tree_summarizer_status(&cfg, ns)
        .await
        .expect("status");
    assert_eq!(status.value["total_nodes"], 6);
    assert_eq!(status.value["depth"], 5);

    let query = tree_runtime_rpc::tree_summarizer_query(&cfg, ns, Some("2026/05/29"))
        .await
        .expect("query day");
    assert_eq!(query.value["children"].as_array().unwrap().len(), 2);

    runtime_store::buffer_write(
        &cfg,
        ns,
        "preserve me through rebuild",
        &Utc.with_ymd_and_hms(2026, 5, 29, 12, 0, 0).unwrap(),
        None,
    )
    .expect("write rebuild buffer");
    let rebuild_provider = ScriptedProvider::new([
        "rebuilt day summary",
        "rebuilt month summary",
        "rebuilt year summary",
        "rebuilt root summary",
    ]);
    let rebuilt = engine::rebuild_tree(&cfg, &rebuild_provider, "test-model", ns)
        .await
        .expect("rebuild tree");
    assert_eq!(rebuilt.total_nodes, 6);
    assert_eq!(runtime_store::buffer_read(&cfg, ns).unwrap().len(), 1);
}

#[tokio::test]
async fn bucket_seal_deferred_and_fallback_paths_preserve_buffers_and_labels() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg = config_in(&tmp);
    let tree = get_or_create_tree(&cfg, TreeKind::Source, "slack:#round14").expect("tree");

    let ts = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
    let small = LeafRef {
        chunk_id: "missing-small".into(),
        token_count: 10,
        timestamp: ts,
        content: "small body".into(),
        entities: vec![],
        topics: vec![],
        score: 0.1,
    };
    assert!(!append_leaf_deferred(&cfg, &tree, &small).expect("append small"));
    assert!(!append_leaf_deferred(&cfg, &tree, &small).expect("append duplicate"));
    let l0 = tree_store::get_buffer(&cfg, &tree.id, 0).expect("l0 buffer");
    assert_eq!(l0.item_ids, vec!["missing-small"]);
    assert_eq!(l0.token_sum, 10);

    let c1 = staged_chunk(&cfg, "slack:#round14", 1, INPUT_TOKEN_BUDGET / 2);
    let c2 = staged_chunk(&cfg, "slack:#round14", 2, INPUT_TOKEN_BUDGET / 2);
    let leaf1 = LeafRef {
        chunk_id: c1.id.clone(),
        token_count: c1.token_count,
        timestamp: c1.created_at,
        content: c1.content.clone(),
        entities: vec!["email:alice@example.com".into()],
        topics: vec!["launch".into()],
        score: 0.7,
    };
    let leaf2 = LeafRef {
        chunk_id: c2.id.clone(),
        token_count: c2.token_count,
        timestamp: c2.created_at,
        content: c2.content.clone(),
        entities: vec!["person:bob".into()],
        topics: vec!["cleanup".into()],
        score: 0.8,
    };
    assert!(!append_leaf_deferred(&cfg, &tree, &leaf1).expect("append leaf1"));
    assert!(append_leaf_deferred(&cfg, &tree, &leaf2).expect("append leaf2"));

    let seeded = tree_store::get_buffer(&cfg, &tree.id, 0).expect("seeded buffer");
    assert!(seeded.item_ids.iter().any(|id| id == &c1.id));
    assert!(seeded.item_ids.iter().any(|id| id == &c2.id));

    let sealed = append_leaf(&cfg, &tree, &leaf2, &LabelStrategy::Empty)
        .await
        .expect("fallback seal");
    assert_eq!(sealed.len(), 1);
    let summary = tree_store::get_summary(&cfg, &sealed[0])
        .expect("summary read")
        .expect("summary exists");
    assert_eq!(summary.level, 1);
    assert!(summary.content.contains("raw coverage chunk"));
    assert!(summary.entities.is_empty());
    assert!(summary.topics.is_empty());

    let after_l0 = tree_store::get_buffer(&cfg, &tree.id, 0).expect("after l0");
    assert!(after_l0.is_empty());
    let parent = tree_store::get_buffer(&cfg, &tree.id, 1).expect("parent buffer");
    assert_eq!(parent.item_ids, sealed);
}

#[tokio::test]
async fn composio_providers_sync_state_and_bus_surfaces_cover_read_write_edges() {
    let _lock = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", tmp.path());
    let _triage = EnvVarGuard::set_str("OPENHUMAN_TRIGGER_TRIAGE_DISABLED", "yes");

    let matrix = capability_matrix();
    assert!(matrix
        .iter()
        .any(|cap| cap.toolkit == "gmail" && cap.native_provider));
    assert!(matrix
        .iter()
        .any(|cap| cap.toolkit == "googlecalendar" && cap.curated_tools));
    let ready = agent_ready_toolkits();
    assert!(ready.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(ready.contains(&"gmail"));

    let gmail_catalog = catalog_for_toolkit("gmail").expect("gmail catalog");
    assert_eq!(
        find_curated(gmail_catalog, "gmail_fetch_emails").map(|c| c.scope),
        Some(ToolScope::Read)
    );
    assert_eq!(
        toolkit_from_slug("MICROSOFT_TEAMS_SEND_MESSAGE").as_deref(),
        Some("microsoft_teams")
    );
    assert_eq!(classify_unknown("GMAIL_DELETE_DRAFT"), ToolScope::Admin);
    assert_eq!(classify_unknown("NOTION_CREATE_PAGE"), ToolScope::Write);
    assert!(toolkit_has_scope("gmail", ToolScope::Read));

    let read_only = UserScopePref {
        read: true,
        write: false,
        admin: false,
    };
    assert!(is_action_visible_with_pref(
        "GMAIL_FETCH_EMAILS",
        &read_only
    ));
    assert!(!is_action_visible_with_pref("GMAIL_SEND_EMAIL", &read_only));

    let mut budget = DailyBudget {
        date: "1999-01-01".into(),
        requests_used: 499,
        limit: 500,
    };
    assert_eq!(budget.remaining(), 500);
    budget.record_requests(2);
    assert_eq!(budget.requests_used, 2);
    assert!(!budget.is_exhausted());

    let mut state = SyncState::new("gmail", "conn-round14");
    assert_eq!(state.budget_remaining(), 500);
    state.record_requests(500);
    assert!(state.budget_exhausted());
    state.mark_synced("msg-1");
    state.advance_cursor("1700000000000");
    state.set_last_seen_id("msg-2");
    state.set_last_sync_at_ms(1_700_000_000_123);
    assert!(state.is_synced("msg-1"));
    assert_eq!(state.cursor.as_deref(), Some("1700000000000"));
    assert_eq!(
        extract_item_id(
            &json!({"data": {"message": {"id": " nested-id "}}, "id": "fallback"}),
            &["data.message.id", "id"]
        )
        .as_deref(),
        Some("nested-id")
    );

    let trigger = ComposioTriggerSubscriber::new();
    assert_eq!(trigger.name(), "composio::trigger");
    assert_eq!(trigger.domains(), Some(&["composio"][..]));
    trigger
        .handle(&DomainEvent::ComposioTriggerReceived {
            toolkit: "gmail".into(),
            trigger: "GMAIL_NEW_GMAIL_MESSAGE".into(),
            metadata_id: "meta-1".into(),
            metadata_uuid: "uuid-1".into(),
            payload: json!({"subject": "coverage"}),
        })
        .await;

    let config_changed = ComposioConfigChangedSubscriber::new();
    assert_eq!(config_changed.name(), "composio::config_changed");
    config_changed
        .handle(&DomainEvent::ComposioConfigChanged {
            mode: "direct".into(),
            api_key_set: true,
        })
        .await;
}

#[tokio::test]
async fn default_composio_provider_hooks_return_expected_noop_shapes() {
    struct MinimalProvider;

    #[async_trait]
    impl ComposioProvider for MinimalProvider {
        fn toolkit_slug(&self) -> &'static str {
            "round14"
        }

        fn sync_interval_secs(&self) -> Option<u64> {
            None
        }

        fn curated_tools(&self) -> Option<&'static [CuratedTool]> {
            Some(&[CuratedTool {
                slug: "ROUND14_READ",
                scope: ToolScope::Read,
            }])
        }

        async fn fetch_user_profile(
            &self,
            ctx: &ProviderContext,
        ) -> Result<ProviderUserProfile, String> {
            Ok(ProviderUserProfile {
                toolkit: ctx.toolkit.clone(),
                connection_id: ctx.connection_id.clone(),
                display_name: Some("Round Fourteen".into()),
                email: Some("round14@example.com".into()),
                username: Some("round14".into()),
                avatar_url: None,
                profile_url: None,
                extras: json!({"source": "test"}),
            })
        }

        async fn sync(
            &self,
            ctx: &ProviderContext,
            reason: SyncReason,
        ) -> Result<SyncOutcome, String> {
            Ok(SyncOutcome {
                toolkit: ctx.toolkit.clone(),
                connection_id: ctx.connection_id.clone(),
                reason: reason.as_str().into(),
                items_ingested: 3,
                started_at_ms: 10,
                finished_at_ms: 25,
                summary: "synced".into(),
                details: json!({"reason": reason.as_str()}),
            })
        }
    }

    let tmp = TempDir::new().expect("tempdir");
    let cfg = Arc::new(config_in(&tmp));
    let ctx = ProviderContext {
        config: cfg,
        toolkit: "round14".into(),
        connection_id: Some("conn-round14".into()),
        usage: Default::default(),
        max_items: None,
        sync_depth_days: None,
    };
    let provider = MinimalProvider;
    assert_eq!(provider.sync_interval_secs(), None);
    assert_eq!(provider.curated_tools().unwrap()[0].scope.as_str(), "read");
    let facets_written = provider.identity_set(&provider.fetch_user_profile(&ctx).await.unwrap());
    assert!(facets_written <= 4);

    let filter = TaskFetchFilter {
        max: 0,
        ..TaskFetchFilter::default()
    };
    assert_eq!(filter.effective_max(), 25);
    let err = provider.fetch_tasks(&ctx, &filter).await.unwrap_err();
    assert!(err.contains("provider has no task-fetch surface"));

    let mut data = json!({"ok": true});
    provider.post_process_action_result("ROUND14_READ", None, &mut data);
    assert_eq!(data, json!({"ok": true}));
    provider
        .on_trigger(&ctx, "ROUND14_TRIGGER", &json!({"ok": true}))
        .await
        .expect("default trigger no-op");

    let profile = provider.fetch_user_profile(&ctx).await.expect("profile");
    assert_eq!(profile.email.as_deref(), Some("round14@example.com"));
    let sync = provider
        .sync(&ctx, SyncReason::Manual)
        .await
        .expect("sync outcome");
    assert_eq!(sync.elapsed_ms(), 15);

    let extracted = ExtractedEntities {
        entities: vec![
            openhuman_core::openhuman::memory_tree::score::extract::ExtractedEntity {
                kind: EntityKind::Email,
                text: "Round14@Example.COM".into(),
                span_start: 0,
                span_end: 19,
                score: 0.9,
            },
            openhuman_core::openhuman::memory_tree::score::extract::ExtractedEntity {
                kind: EntityKind::Person,
                text: "Round Fourteen".into(),
                span_start: 20,
                span_end: 34,
                score: 0.7,
            },
        ],
        topics: vec![],
        llm_importance: Some(0.5),
        llm_importance_reason: Some("coverage fixture".into()),
    };
    let canonical = canonicalise(&extracted);
    assert!(canonical
        .iter()
        .any(|entity| entity.canonical_id == "email:round14@example.com"));
    assert!(approx_token_count("one two three four") > 0);
}
