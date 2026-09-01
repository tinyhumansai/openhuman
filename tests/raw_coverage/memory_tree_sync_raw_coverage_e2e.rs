//! Focused raw integration coverage for memory-tree and memory-sync modules.
//!
//! This suite is intentionally hermetic: every test uses a temp workspace and
//! any provider behavior is supplied by small in-process stubs. Run with
//! `--test-threads=1` because config/env and a few registries are global.

use std::ffi::OsString;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use serde_json::json;
use tempfile::TempDir;

use openhuman_core::core::events::DomainEvent;
use tinybus::EventHandler;
use openhuman_core::openhuman::config::Config;
use tinymemory_core::store::chunks::store::upsert_chunks;
use tinymemory_core::store::chunks::types::{
    approx_token_count, chunk_id, Chunk, Metadata, SourceKind as ChunkSourceKind, SourceRef,
};
use tinymemory_core::store::content;
use tinymemory_core::store::trees::types::TreeKind;
use tinymemory_core::store::trees::types::INPUT_TOKEN_BUDGET;
use openhuman_core::openhuman::memory::sync::composio::bus::{
    ComposioConfigChangedSubscriber, ComposioTriggerSubscriber,
};
// `sync_state` moved off `memory::sync::composio::providers` (the deleted
// engine registry's former home) onto the contract crate directly — this
// data (a per-connection cursor/dedup-set/budget) was always pure `serde`
// vocabulary shared by both sides of the module boundary, never engine
// behaviour, so it re-exports unchanged. See
// `integrations::composio::providers`'s module docs for the fuller account of
// what moved where.
use tinymemory_api::composio::state::{extract_item_id, DailyBudget, SyncState};
use openhuman_core::openhuman::integrations::composio::providers::{
    agent_ready_toolkits, catalog_for_toolkit, classify_unknown, find_curated, has_native_provider,
    is_action_visible_with_pref, toolkit_from_slug, toolkit_has_scope, ToolScope, UserScopePref,
};
use tinymemory_core::tree::score::extract::{EntityKind, ExtractedEntities};
use tinymemory_core::tree::score::resolver::canonicalise;
use tinymemory_core::tree::tree::bucket_seal::append_leaf;
use tinymemory_core::tree::tree::{
    append_leaf_deferred, get_or_create_tree, store as tree_store, LabelStrategy, LeafRef,
};
// As above: the host re-export is gone, the engine is named directly (#5560).
//
// `tree_runtime::rpc` is deliberately **not** imported here any more. Its five
// handlers answer from the loaded module's store, and a case that mixed them
// with these engine calls would be driving two stores that share no state — see
// `tree_runtime_engine_rpc_and_walk_cover_success_and_edge_paths`.
use tinymemory_core::tree::tree_runtime::{engine, store as runtime_store};
use tinyinference::model::{ChatModel, ModelRequest, ModelResponse};

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

static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn config_in(tmp: &TempDir) -> Config {
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    cfg.config_path = tmp.path().join("config.toml");
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
    tinymemory_core::store::chunks::store::with_connection(cfg, |conn| {
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
impl ChatModel<()> for ScriptedProvider {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        let response = self
            .responses
            .lock()
            .unwrap()
            .pop()
            .unwrap_or_else(|| "fallback scripted summary".to_string());
        Ok(ModelResponse::assistant(response))
    }
}

/// The engine's summarisation walk: buffer → hour leaves → propagated
/// ancestors → rebuild, end to end against a scripted summariser.
///
/// # One door, and why it is the engine's (#5560)
///
/// This case used to seed and read through `tree_runtime::rpc`'s handlers while
/// folding through `engine::run_summarization`, and after `d2697f00a` those are
/// two different stores: the handlers answer from the **loaded module's** engine
/// over the bus, `engine::` runs the copy the `[dev-dependencies]` entry links
/// into this binary. The fold therefore drained a buffer the ingests had never
/// written to and answered `None` — "last hour node".
///
/// The subject here is the walk, not the RPC envelope, so the whole case takes
/// the engine door. That is also the only door it *can* take: `run_summarization`
/// takes an explicit provider, and the contract's `runtime_summarize` does not —
/// the fold runs on the driver's own chat provider, deliberately, so the
/// [`ScriptedProvider`] below cannot cross the bus. Routing this through
/// `tree_summarizer_run` would mean a real summarisation model in a hermetic
/// suite.
///
/// Nothing is left uncovered by that choice. The handlers' side of the same
/// ground is asserted where it belongs — against a bound driver — by
/// `memory::tree::tree_runtime::ops_tests`
/// (`tree_summarizer_status_reports_populated_tree_details` pins the same six
/// nodes at depth five, `tree_summarizer_query_returns_node_and_children` the
/// same node-plus-children envelope) and, module-routed, by
/// `memory_tree_memory_round23_raw_coverage_e2e::
/// tree_runtime_rpc_and_registered_handlers_cover_status_and_errors`.
#[tokio::test]
async fn tree_runtime_engine_rpc_and_walk_cover_success_and_edge_paths() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg = config_in(&tmp);
    let ns = "round14-team";

    let first_ts = Utc.with_ymd_and_hms(2026, 5, 29, 10, 15, 0).unwrap();
    let second_ts = Utc.with_ymd_and_hms(2026, 5, 29, 11, 45, 0).unwrap();
    let first_path = runtime_store::buffer_write(
        &cfg,
        ns,
        "deployment notes mention Alice and the launch room",
        &first_ts,
        Some(&json!({"source": "round14"})),
    )
    .expect("buffer first");
    runtime_store::buffer_write(
        &cfg,
        ns,
        "follow-up notes mention Bob and post-launch cleanup",
        &second_ts,
        None,
    )
    .expect("buffer second");
    // The two ingests are what the fold consumes, so observe them before it
    // runs: this is the half `tree_summarizer_ingest` used to stand in for, and
    // it is the half that silently stopped being observed once the handler
    // started writing to the module's store instead of this one. Metadata is
    // read off the file rather than out of `buffer_read`, which strips the
    // frontmatter it is carried in.
    let on_disk = std::fs::read_to_string(&first_path).expect("buffer file exists");
    assert!(on_disk.contains("deployment notes mention Alice"));
    assert!(on_disk.contains("\"source\":\"round14\""));
    let buffered = runtime_store::buffer_read(&cfg, ns).expect("buffer read before drain");
    assert_eq!(buffered.len(), 2);
    assert!(buffered
        .iter()
        .any(|(_, body)| body.contains("post-launch cleanup")));

    let provider = ScriptedProvider::new([
        "hour 10 summary about Alice",
        "hour 11 summary about Bob",
        "rebuilt hour 10",
        "rebuilt hour 11",
    ]);
    let last = engine::run_summarization(&cfg, &provider, ns, Utc::now())
        .await
        .expect("run summarization")
        .expect("last hour node");
    assert_eq!(last.node_id, "2026/05/29/11");
    assert!(runtime_store::buffer_read(&cfg, ns)
        .expect("buffer read after drain")
        .is_empty());

    let status = runtime_store::get_tree_status(&cfg, ns).expect("status");
    assert_eq!(status.namespace, ns);
    assert_eq!(status.total_nodes, 6);
    assert_eq!(status.depth, 5);

    // What `tree_summarizer_query` renders as `{node, children}`, read as the
    // two store calls the handler now makes over the bus.
    let day = runtime_store::read_node(&cfg, ns, "2026/05/29")
        .expect("read day node")
        .expect("day node exists");
    assert_eq!(day.node_id, "2026/05/29");
    let children = runtime_store::read_children(&cfg, ns, "2026/05/29").expect("read day children");
    assert_eq!(children.len(), 2);

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
    let rebuilt = engine::rebuild_tree(&cfg, &rebuild_provider, ns)
        .await
        .expect("rebuild tree");
    assert_eq!(rebuilt.total_nodes, 6);
    assert_eq!(runtime_store::buffer_read(&cfg, ns).unwrap().len(), 1);
}

#[tokio::test]
async fn bucket_seal_deferred_and_fallback_paths_preserve_buffers_and_labels() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg = config_in(&tmp);
    openhuman_core::openhuman::memory::host_impls::install_memory_host_seams(
        std::sync::Arc::new(cfg.clone()),
    );
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

    // `capability_matrix()` — a pure host-side function that used to build
    // this table from the engine's provider registry — was deleted by
    // tinymemory v1.13.4 with no replacement here; `composio_list_capabilities`
    // now answers the equivalent RPC directly from the connectors module's
    // `ListCapabilities` member (module-mediated, not testable network-free
    // from this crate). What the matrix reported per toolkit is still
    // answerable from the two pure functions that fed it, though:
    // `has_native_provider` and `catalog_for_toolkit(..).is_some()`.
    assert!(has_native_provider("gmail"));
    assert!(catalog_for_toolkit("googlecalendar").is_some());
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

// `default_composio_provider_hooks_cover_defaults_and_sync_preconditions`
// used to implement a `MinimalProvider: ComposioProvider` and exercise the
// trait's *default* method bodies: `identity_set`'s facet-count return,
// `fetch_tasks`'s "provider has no task-fetch surface" default error,
// `post_process_action_result`'s no-op default, `on_trigger`'s no-op
// default, and `sync`'s "memory client is not ready" precondition check.
//
// `ComposioProvider` itself — trait, default methods included — is one of
// the types tinymemory v1.13.4 deleted outright with the rest of the
// in-process Composio pipeline (see
// `crate::openhuman::integrations::composio::providers`'s module docs). It
// did not move to a replacement inside this crate: reaching a connected
// account now needs a credential this crate must not hold, so there is no
// trait left to implement a minimal provider against, default methods
// included. The nearest current behaviour — `run_sync_pass` refusing when no
// connectors module is loaded, and `task_sources::pipeline::fetch_tasks_unavailable`
// refusing task-board fetch for every toolkit — is already covered by
// `composio_get_user_profile_refuses_cleanly_without_a_loaded_module`-style
// tests elsewhere in this suite (see
// memory_sync_round23_raw_coverage_e2e.rs and
// json_rpc_e2e.rs::json_rpc_task_sources_fetch_pipeline_e2e), so this test
// keeps only the entity-canonicalisation coverage below, which has nothing
// to do with Composio and is untouched by the deletion. The trait-default
// coverage above is a genuine gap with no local equivalent — flagged in the
// migration report rather than papered over.
#[tokio::test]
async fn memory_tree_entity_canonicalisation_covers_email_and_person_kinds() {
    let extracted = ExtractedEntities {
        entities: vec![
            tinymemory_core::tree::score::extract::ExtractedEntity {
                kind: EntityKind::Email,
                text: "Round14@Example.COM".into(),
                span_start: 0,
                span_end: 19,
                score: 0.9,
            },
            tinymemory_core::tree::score::extract::ExtractedEntity {
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
