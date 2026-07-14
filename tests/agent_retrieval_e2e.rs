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
//! The orchestrator agent.toml entry registering `call_memory_agent` is
//! covered by [`orchestrator_lists_memory_tree_tools`] — that catches a
//! regression where the tool wrapper exists but the orchestrator can't see
//! it.

use chrono::{TimeZone, Utc};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::memory::ingest_pipeline::{ingest_chat, ingest_email};
use openhuman_core::openhuman::memory::jobs::drain_until_idle;
use openhuman_core::openhuman::memory_sync::canonicalize::chat::{ChatBatch, ChatMessage};
use openhuman_core::openhuman::memory_sync::canonicalize::email::{EmailMessage, EmailThread};
use openhuman_core::openhuman::tools::{
    MemoryTreeFetchLeavesTool, MemoryTreeSearchEntitiesTool, Tool,
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
                list_unsubscribe: None,
            },
            EmailMessage {
                from: "bob@example.com".into(),
                to: vec!["alice@example.com".into()],
                cc: vec![],
                subject: "Re: Phoenix migration plan".into(),
                sent_at: Utc.timestamp_millis_opt(1_700_000_060_000).unwrap(),
                body: "Confirmed — I'll review the phoenix runbook tonight.".into(),
                source_ref: Some("<phoenix-2@example.com>".into()),
                list_unsubscribe: None,
            },
        ],
    }
}

/// The orchestrator reaches `agent_memory` ON-DEMAND, not via an eager
/// pre-turn pre-fetch. `agent_memory` is listed in the `[subagents]` allowlist
/// (synthesised into a `delegate_retrieve_memory` tool), so the orchestrator
/// walks the memory tree only when a message needs it — instead of spawning a
/// full memory subagent before every turn.
///
/// History: #1141 consolidated 6 `memory_tree_*` tools into `memory_tree`;
/// the agent_memory domain then unified `memory_tree` + `query_memory`
/// behind `call_memory_agent`; a later revision made it an eager
/// `trigger_memory_agent = "always"` pre-fetch. This contract reverts the
/// eager pre-fetch to an on-demand delegation (the memory agent is heavy and
/// most turns don't need a deep tree walk).
#[test]
fn orchestrator_reaches_memory_agent_on_demand() {
    let toml = include_str!("../src/openhuman/agent_registry/agents/orchestrator/agent.toml");
    // Eager pre-fetch must be gone.
    assert!(
        !toml
            .lines()
            .map(str::trim)
            .any(|line| line == "trigger_memory_agent = \"always\""),
        "orchestrator must NOT eagerly pre-fetch the memory agent — retrieval is on-demand"
    );
    // The on-demand route: `agent_memory` in the subagents allowlist.
    assert!(
        toml.lines()
            .map(str::trim)
            .any(|line| line == "\"agent_memory\"" || line == "\"agent_memory\","),
        "orchestrator must list `agent_memory` in its subagents allowlist for on-demand retrieval"
    );
    // The orchestrator reaches the memory agent through delegation, not the
    // direct `call_memory_agent` tool (that tool is forbidden on the
    // orchestrator — see loader::tests::orchestrator_has_chat_hint_and_named_tools).
    let has_call_memory_agent = toml
        .lines()
        .map(str::trim)
        .any(|line| line == "\"call_memory_agent\"" || line == "\"call_memory_agent\",");
    assert!(
        !has_call_memory_agent,
        "orchestrator agent.toml must not expose the 'call_memory_agent' tool"
    );
    // Simple recall/store operations stay direct so they do not pay an agentic
    // round-trip; deep retrieval still routes through the memory subagent.
    for direct_name in [
        "memory_recall",
        "memory_store",
        "save_preference",
        "update_memory_md",
    ] {
        let entry = format!("\"{direct_name}\"");
        let entry_comma = format!("\"{direct_name}\",");
        let direct_tool_present = toml
            .lines()
            .map(str::trim)
            .any(|line| line == entry || line == entry_comma);
        assert!(
            direct_tool_present,
            "orchestrator agent.toml must list direct memory tool '{direct_name}'"
        );
    }
    // Verify the superseded tree/query tool names are gone.
    for old_name in [
        "memory_tree",
        "query_memory",
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
            "orchestrator agent.toml must NOT list legacy memory tool '{old_name}'"
        );
    }
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
                    list_unsubscribe: None,
                },
                EmailMessage {
                    from: "alice@example.com".into(),
                    to: vec!["pm@example.com".into()],
                    cc: vec![],
                    subject: "Re: Q3 roadmap decision".into(),
                    sent_at: Utc.timestamp_millis_opt(1_710_000_060_000).unwrap(),
                    body: "Confirmed. alice@example.com will own the Phoenix delivery.".into(),
                    source_ref: Some("<q3-roadmap-2@example.com>".into()),
                    list_unsubscribe: None,
                },
            ],
        },
    )
    .await
    .expect("ingest_email must succeed");

    drain_until_idle(&cfg).await.expect("queue must drain");

    let _ws_guard = set_workspace_env(&tmp);

    // List the ingested chunks directly to get leaf chunk ids with their refs.
    let chunks = openhuman_core::openhuman::memory_store::chunks::store::list_chunks(
        &cfg,
        &openhuman_core::openhuman::memory_store::chunks::store::ListChunksQuery::default(),
    )
    .expect("list_chunks must not error");

    assert!(!chunks.is_empty(), "ingest must produce at least one chunk");

    // Collect the first couple of leaf chunk ids.
    let leaf_ids: Vec<String> = chunks
        .iter()
        .map(|chunk| chunk.id.clone())
        .take(2)
        .collect();

    assert!(
        !leaf_ids.is_empty(),
        "at least one leaf chunk required for fetch_leaves provenance test"
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
