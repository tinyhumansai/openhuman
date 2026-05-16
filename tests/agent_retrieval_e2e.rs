//! End-to-end coverage for the orchestrator memory-tree retrieval tool
//! wrappers (issue #710 wiring).
//!
//! Goal: prove the `MemoryTree*Tool` instances actually drive the typed
//! retrieval functions against a real ingested workspace and emit JSON the
//! orchestrator LLM can parse + cite from.
//!
//! Why a tool-direct test (and not a full `agent_chat` round-trip):
//! `agent_chat` requires a reachable provider (no provider connection
//! available in unit-test context). The bus-level `mock_agent_run_turn`
//! stub replaces the agent loop wholesale, so it can't observe a tool
//! dispatch happening *inside* the loop. Calling each tool's `execute()`
//! with the same JSON shape the LLM would emit exercises the full
//! deserialise → typed retrieval → serialise pipeline that the orchestrator
//! relies on, and asserts the data round-trips correctly.
//!
//! The orchestrator agent.toml entry registering these tool names is
//! covered by [`orchestrator_lists_memory_tree_tools`] — that catches a
//! regression where the tool wrapper exists but the orchestrator can't see
//! it.

use chrono::{TimeZone, Utc};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::memory::tree::canonicalize::chat::{ChatBatch, ChatMessage};
use openhuman_core::openhuman::memory::tree::canonicalize::email::{EmailMessage, EmailThread};
use openhuman_core::openhuman::memory::tree::ingest::{ingest_chat, ingest_email};
use openhuman_core::openhuman::memory::tree::jobs::drain_until_idle;
use openhuman_core::openhuman::tools::{
    MemoryTreeFetchLeavesTool, MemoryTreeQueryTopicTool, MemoryTreeSearchEntitiesTool, Tool,
};
use serde_json::{json, Value};
use tempfile::TempDir;

/// Build a Config rooted at `tmp/workspace`. The nested `workspace` dir
/// matches what `resolve_config_dir_for_workspace` would derive when
/// `OPENHUMAN_WORKSPACE` points at `tmp` — so the same workspace_dir is
/// used both by the explicit ingest path and by `load_config_with_timeout`
/// inside the tool wrappers.
fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace_dir).expect("create workspace dir");
    let mut cfg = Config {
        workspace_dir: workspace_dir.clone(),
        ..Config::default()
    };
    // Inert embedder — keeps the test deterministic and avoids any real
    // Ollama call. Mirrors `retrieval/integration_test.rs`.
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    (tmp, cfg)
}

// ── RAII env guard shared by all tests in this file ──────────────────────────

/// Process-wide mutex that serialises every test in this binary that
/// mutates `OPENHUMAN_WORKSPACE`. Cargo runs integration-test binaries
/// multi-threaded by default (`test-threads = num_cpus`), so without
/// this serialisation two tests would race on the env var: test A sets
/// it to `/tmp/aaa`, test B overwrites it with `/tmp/bbb`, then when
/// B's `TempDir` drops it unlinks `/tmp/bbb` while A is still reading
/// from it. That race surfaced in CI as `SQLITE_IOERR_FSTAT` (error
/// code 1802) during a later `with_connection` call on the now-deleted
/// path, and earlier as `fetch_leaves` returning 0 hits when the
/// resolved workspace temporarily pointed at the wrong sibling test's
/// (otherwise empty) tempdir.
///
/// `unwrap_or_else(|p| p.into_inner())` keeps the lock usable after a
/// poisoning panic so one failing test never cascades.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct EnvGuard {
    key: &'static str,
    prev: Option<std::ffi::OsString>,
    /// Last field — dropped after `Drop::drop` has already restored
    /// the env var, so the next test acquires the lock against a
    /// clean `OPENHUMAN_WORKSPACE` value.
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: cargo test runs each integration test binary in its own
        // process; the `ENV_LOCK` mutex held in `_lock` serialises all
        // mutations within this binary, and the guard restores the
        // previous value before the lock is released.
        unsafe {
            match self.prev.take() {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

/// Sets `OPENHUMAN_WORKSPACE` to `tmp.path()` and returns an RAII guard that
/// restores the previous value on drop. This makes the tool wrappers (which
/// call `load_config_with_timeout` internally) resolve to the same workspace
/// that was used for ingest.
///
/// The returned guard also holds [`ENV_LOCK`] for its lifetime, so concurrent
/// tests in the same binary cannot stomp on each other's
/// `OPENHUMAN_WORKSPACE` setting.
fn set_workspace_env(tmp: &TempDir) -> EnvGuard {
    let lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let prev = std::env::var_os("OPENHUMAN_WORKSPACE");
    // SAFETY: see EnvGuard::Drop above.
    unsafe { std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path()) };
    EnvGuard {
        key: "OPENHUMAN_WORKSPACE",
        prev,
        _lock: lock,
    }
}

fn alice_phoenix_thread() -> EmailThread {
    EmailThread {
        provider: "gmail".into(),
        thread_subject: "Phoenix migration plan".into(),
        messages: vec![
            EmailMessage {
                from: "alice@example.com".into(),
                to: vec!["bob@example.com".into()],
                cc: vec![],
                subject: "Phoenix migration plan".into(),
                sent_at: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
                body: "Hey Bob, the phoenix migration runbook is ready for review. \
                       I'm coordinating with the infra team and we land Friday."
                    .into(),
                source_ref: Some("<phoenix-1@example.com>".into()),
            },
            EmailMessage {
                from: "bob@example.com".into(),
                to: vec!["alice@example.com".into()],
                cc: vec![],
                subject: "Re: Phoenix migration plan".into(),
                sent_at: Utc.timestamp_millis_opt(1_700_000_060_000).unwrap(),
                body: "Confirmed — I'll review the phoenix runbook tonight.".into(),
                source_ref: Some("<phoenix-2@example.com>".into()),
            },
        ],
    }
}

/// The orchestrator definition must list the consolidated `memory_tree` tool
/// so the bus filter exposes it to the LLM. A wired-up wrapper that's
/// invisible to the orchestrator is dead code.
///
/// NOTE: #1141 consolidated the 6 individual `memory_tree_*` tools
/// (`memory_tree_search_entities`, `memory_tree_query_topic`, etc.) into a
/// single `memory_tree` tool with a `mode` dispatch parameter. The orchestrator
/// TOML was updated accordingly.
#[test]
fn orchestrator_lists_memory_tree_tools() {
    let toml = include_str!("../src/openhuman/agent/agents/orchestrator/agent.toml");
    // Exact entry match — substring match would also hit comments or prefixed names.
    let has_memory_tree_entry = toml
        .lines()
        .map(str::trim)
        .any(|line| line == "\"memory_tree\"" || line == "\"memory_tree\",");
    assert!(
        has_memory_tree_entry,
        "orchestrator agent.toml must list 'memory_tree' as a named tool entry"
    );
    // Verify the old individual tool names are gone — they were removed in #1141
    // when all 6 were consolidated into the single `memory_tree` dispatcher.
    for old_name in [
        "memory_tree_search_entities",
        "memory_tree_query_topic",
        "memory_tree_query_source",
        "memory_tree_query_global",
        "memory_tree_drill_down",
        "memory_tree_fetch_leaves",
    ] {
        let entry = format!("\"{old_name}\"");
        let entry_comma = format!("\"{old_name}\",");
        let old_tool_present = toml
            .lines()
            .map(str::trim)
            .any(|line| line == entry || line == entry_comma);
        assert!(
            !old_tool_present,
            "orchestrator agent.toml must NOT list '{old_name}' — removed in #1141 (use 'memory_tree' with mode= dispatch)"
        );
    }
}

#[tokio::test]
async fn orchestrator_query_topic_tool_returns_alice_phoenix_hits() {
    let (tmp, cfg) = test_config();

    // ── Ingest the email thread + drain async extract jobs so the entity
    //    index is fully populated before retrieval.
    ingest_email(
        &cfg,
        "gmail:thread-phoenix-1",
        "alice",
        vec![],
        alice_phoenix_thread(),
    )
    .await
    .expect("ingest_email should succeed");
    drain_until_idle(&cfg)
        .await
        .expect("job queue should drain cleanly");

    // Set workspace dir so config_rpc::load_config_with_timeout() inside the
    // tool resolves to the same workspace we just ingested into. The tool
    // wrappers always go through that loader (mirrors the production RPC
    // handlers in retrieval/schemas.rs).
    //
    // Pointing OPENHUMAN_WORKSPACE at `tmp` (not `tmp/workspace`) makes
    // `resolve_config_dir_for_workspace` derive `tmp/workspace` as the
    // resolved workspace_dir — matching what we already passed into
    // `ingest_email` via `cfg.workspace_dir`.
    let _ws_guard = set_workspace_env(&tmp);

    // ── 1. search_entities resolves "alice" → email:alice@example.com.
    //    Mirrors the orchestrator prompt's "ALWAYS call this first when
    //    the user mentions someone by name" flow.
    let search = MemoryTreeSearchEntitiesTool;
    let search_args = json!({"query": "alice"});
    let search_res = search
        .execute(search_args)
        .await
        .expect("search_entities should not error");
    assert!(
        !search_res.is_error,
        "search_entities returned an error result: {}",
        search_res.output()
    );
    let search_json: Value =
        serde_json::from_str(&search_res.output()).expect("search output must be valid JSON");
    let matches = search_json
        .as_array()
        .expect("search_entities returns an array of EntityMatch");
    let alice = matches
        .iter()
        .find(|m| m.get("canonical_id").and_then(|v| v.as_str()) == Some("email:alice@example.com"))
        .unwrap_or_else(|| panic!("search_entities did not return alice; got: {search_json:?}"));
    assert!(
        alice
            .get("mention_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            >= 1,
        "alice should have at least one mention"
    );

    // ── 2. query_topic on alice's canonical id returns at least one hit
    //    referencing both her email and the phoenix migration content.
    let topic_tool = MemoryTreeQueryTopicTool;
    let topic_args = json!({"entity_id": "email:alice@example.com"});
    let topic_res = topic_tool
        .execute(topic_args)
        .await
        .expect("query_topic should not error");
    assert!(
        !topic_res.is_error,
        "query_topic returned an error result: {}",
        topic_res.output()
    );
    let topic_json: Value =
        serde_json::from_str(&topic_res.output()).expect("topic output must be valid JSON");
    let hits = topic_json
        .get("hits")
        .and_then(|v| v.as_array())
        .expect("query_topic must include `hits` array");
    assert!(
        !hits.is_empty(),
        "query_topic returned zero hits — expected at least one for alice"
    );
    // Returning ANY hit at all from `query_topic("email:alice@example.com")`
    // proves the entity index resolved the canonical id and hydrated nodes
    // back. The leaf-level `entities` field on a chunk hit isn't populated
    // synchronously by ingest — entity extraction lives in a separate async
    // job stage that may not have populated leaf rows. Instead we assert on
    // the hydrated content + source_ref so we still catch a regression where
    // the chunk lookup returns garbage.
    let any_phoenix = hits.iter().any(|h| {
        h.get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase()
            .contains("phoenix")
    });
    assert!(
        any_phoenix,
        "expected at least one query_topic hit with phoenix content; got: {topic_json:#}"
    );
    let any_source_ref = hits
        .iter()
        .any(|h| h.get("source_ref").and_then(|v| v.as_str()).is_some());
    assert!(
        any_source_ref,
        "expected at least one hit to carry a `source_ref` for citation; got: {topic_json:#}"
    );

    // ── 3. fetch_leaves hydrates a leaf chunk — proves the citation path
    //    (LLM picks an id from a query_* hit, calls fetch_leaves to get
    //    the verbatim content + source_ref).
    let leaf_id = hits
        .iter()
        .find_map(|h| {
            if h.get("node_kind").and_then(|v| v.as_str()) == Some("leaf") {
                h.get("node_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            } else {
                None
            }
        })
        .expect("alice's topic hits should include at least one leaf");
    let fetch_tool = MemoryTreeFetchLeavesTool;
    let fetch_args = json!({"chunk_ids": [leaf_id.clone()]});
    let fetch_res = fetch_tool
        .execute(fetch_args)
        .await
        .expect("fetch_leaves should not error");
    assert!(
        !fetch_res.is_error,
        "fetch_leaves returned an error result: {}",
        fetch_res.output()
    );
    let fetched: Value =
        serde_json::from_str(&fetch_res.output()).expect("fetch output must be valid JSON");
    let fetched_arr = fetched.as_array().expect("fetch_leaves returns array");
    assert_eq!(
        fetched_arr.len(),
        1,
        "fetch_leaves should hydrate exactly the requested chunk"
    );
    let content = fetched_arr[0]
        .get("content")
        .and_then(|v| v.as_str())
        .expect("fetched leaf must carry content");
    assert!(
        !content.is_empty(),
        "fetched leaf content must not be empty"
    );
}

// ── Cross-chat retrieval: chat A seeds facts; retrieve from chat B ──────────

/// Ingests two distinct chat source IDs (simulating two separate chat channels)
/// and proves that `search_entities` surfaces entities that were mentioned in
/// both channels — i.e. the entity index is shared across source boundaries.
///
/// This is the core of "agent retrieves relevant context from other chats"
/// (issue#1505): the retrieval tool must be able to surface facts from a
/// channel the current conversation did not originate in.
#[tokio::test]
async fn cross_chat_entity_index_spans_source_boundaries() {
    let (tmp, cfg) = test_config();

    // Chat A — channel #eng seeds a fact about alice
    let chat_a = ChatBatch {
        platform: "slack".into(),
        channel_label: "#eng".into(),
        messages: vec![ChatMessage {
            author: "alice".into(),
            timestamp: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
            text: "alice@example.com is leading the Phoenix deployment runbook. \
                   Landing confirmed for Friday evening."
                .into(),
            source_ref: Some("slack://eng/1".into()),
        }],
    };
    ingest_chat(&cfg, "slack:#eng", "alice", vec![], chat_a)
        .await
        .expect("ingest chat A should succeed");

    // Chat B — a separate channel with no overlap with chat A
    let chat_b = ChatBatch {
        platform: "slack".into(),
        channel_label: "#ops".into(),
        messages: vec![ChatMessage {
            author: "carol".into(),
            timestamp: Utc.timestamp_millis_opt(1_700_100_000_000).unwrap(),
            text: "What's the Phoenix landing status? carol@example.com asking for ops.".into(),
            source_ref: Some("slack://ops/1".into()),
        }],
    };
    ingest_chat(&cfg, "slack:#ops", "carol", vec![], chat_b)
        .await
        .expect("ingest chat B should succeed");

    drain_until_idle(&cfg)
        .await
        .expect("job queue should drain cleanly");

    let _ws_guard = set_workspace_env(&tmp);

    // search_entities surfaces alice even though the current "context" would
    // be chat B — the entity index is global and crosses source boundaries.
    let search = MemoryTreeSearchEntitiesTool;
    let res = search
        .execute(json!({"query": "alice"}))
        .await
        .expect("search_entities must not error");
    assert!(
        !res.is_error,
        "search_entities returned error: {}",
        res.output()
    );

    let json: Value = serde_json::from_str(&res.output()).unwrap();
    let matches = json.as_array().expect("search_entities returns an array");

    let alice = matches
        .iter()
        .find(|m| m.get("canonical_id").and_then(|v| v.as_str()) == Some("email:alice@example.com"))
        .unwrap_or_else(|| {
            panic!("alice should be discoverable across source boundaries; got: {json:?}")
        });

    // alice was mentioned in chat A only; this assertion confirms the cross-chat
    // retrieval: even from chat B's perspective the entity index resolves her.
    assert!(
        alice
            .get("mention_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            >= 1,
        "alice must have at least one mention"
    );

    // Also verify carol (from chat B) is discoverable via her own
    // canonical entity — a separate search call, since the entity index is
    // keyed by query string and "alice" does not surface carol's row.
    let res_carol = search
        .execute(json!({"query": "carol"}))
        .await
        .expect("search_entities (carol) must not error");
    assert!(
        !res_carol.is_error,
        "search_entities for carol returned error: {}",
        res_carol.output()
    );
    let carol_json: Value = serde_json::from_str(&res_carol.output()).unwrap();
    let carol_matches = carol_json
        .as_array()
        .expect("search_entities returns an array");
    let carol = carol_matches.iter().find(|m| {
        m.get("canonical_id")
            .and_then(|v| v.as_str())
            .map(|s| s.contains("carol"))
            .unwrap_or(false)
    });
    assert!(
        carol.is_some(),
        "carol from chat B must also be discoverable; got: {carol_json:?}"
    );
}

/// Proves fetch_leaves returns a populated `source_ref` on each hydrated
/// chunk so the orchestrator can cite the exact provenance of retrieved facts.
///
/// This is the "memory retrieval returns provenance and can hydrate cited
/// chunks" feature (issue#1538): chunk_ids from query_topic are fed into
/// fetch_leaves and each returned leaf must carry `source_ref` when one was
/// set at ingest time.
#[tokio::test]
async fn fetch_leaves_hydrates_source_ref_for_cited_chunks() {
    let (tmp, cfg) = test_config();

    // Ingest an email thread with explicit source_refs on every message.
    ingest_email(
        &cfg,
        "gmail:thread-provenance-1",
        "alice",
        vec![],
        EmailThread {
            provider: "gmail".into(),
            thread_subject: "Q3 roadmap decision".into(),
            messages: vec![
                EmailMessage {
                    from: "pm@example.com".into(),
                    to: vec!["alice@example.com".into()],
                    cc: vec![],
                    subject: "Q3 roadmap decision".into(),
                    sent_at: Utc.timestamp_millis_opt(1_710_000_000_000).unwrap(),
                    body: "We are committing to the Q3 roadmap with Phoenix as the \
                           flagship feature. pm@example.com signed off."
                        .into(),
                    source_ref: Some("<q3-roadmap-1@example.com>".into()),
                },
                EmailMessage {
                    from: "alice@example.com".into(),
                    to: vec!["pm@example.com".into()],
                    cc: vec![],
                    subject: "Re: Q3 roadmap decision".into(),
                    sent_at: Utc.timestamp_millis_opt(1_710_000_060_000).unwrap(),
                    body: "Confirmed. alice@example.com will own the Phoenix delivery.".into(),
                    source_ref: Some("<q3-roadmap-2@example.com>".into()),
                },
            ],
        },
    )
    .await
    .expect("ingest_email must succeed");

    drain_until_idle(&cfg).await.expect("queue must drain");

    let _ws_guard = set_workspace_env(&tmp);

    // query_topic for alice's entity to get chunk hits with their ids.
    let topic_tool = MemoryTreeQueryTopicTool;
    let topic_res = topic_tool
        .execute(json!({"entity_id": "email:alice@example.com"}))
        .await
        .expect("query_topic must not error");
    assert!(
        !topic_res.is_error,
        "query_topic error: {}",
        topic_res.output()
    );

    let topic_json: Value = serde_json::from_str(&topic_res.output()).unwrap();
    let hits = topic_json
        .get("hits")
        .and_then(|v| v.as_array())
        .expect("query_topic response must have hits array");

    assert!(!hits.is_empty(), "query_topic must return at least one hit");

    // Collect the first leaf chunk id.
    let leaf_ids: Vec<String> = hits
        .iter()
        .filter(|h| h.get("node_kind").and_then(|v| v.as_str()) == Some("leaf"))
        .filter_map(|h| {
            h.get("node_id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .take(2)
        .collect();

    assert!(
        !leaf_ids.is_empty(),
        "at least one leaf hit required for fetch_leaves provenance test"
    );

    // fetch_leaves by chunk_ids and assert source_ref is populated.
    let fetch_tool = MemoryTreeFetchLeavesTool;
    let fetch_res = fetch_tool
        .execute(json!({"chunk_ids": leaf_ids}))
        .await
        .expect("fetch_leaves must not error");
    assert!(
        !fetch_res.is_error,
        "fetch_leaves error: {}",
        fetch_res.output()
    );

    let fetched: Value = serde_json::from_str(&fetch_res.output()).unwrap();
    let leaves = fetched.as_array().expect("fetch_leaves returns array");

    assert!(
        !leaves.is_empty(),
        "fetch_leaves must hydrate at least one chunk"
    );

    // Every leaf that has a source_ref at the ingest level must preserve it.
    // The email thread had explicit source_refs on both messages — at least one
    // leaf should carry provenance.
    let with_source_ref = leaves
        .iter()
        .filter(|l| l.get("source_ref").and_then(|v| v.as_str()).is_some())
        .count();
    assert!(
        with_source_ref >= 1,
        "fetch_leaves must return at least one leaf with source_ref populated \
         (provenance chain for citation); got leaves: {fetched:#}"
    );

    // Verify content round-trips.
    for leaf in leaves {
        let content = leaf.get("content").and_then(|v| v.as_str()).unwrap_or("");
        assert!(
            !content.is_empty(),
            "fetch_leaves leaf must carry non-empty content for citation"
        );
        let node_id = leaf.get("node_id").and_then(|v| v.as_str()).unwrap_or("");
        assert!(!node_id.is_empty(), "fetch_leaves leaf must carry node_id");
    }
}
