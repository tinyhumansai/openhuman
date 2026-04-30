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
use openhuman_core::openhuman::memory::tree::canonicalize::email::{EmailMessage, EmailThread};
use openhuman_core::openhuman::memory::tree::ingest::ingest_email;
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

/// The orchestrator definition must list every memory-tree tool name so
/// the bus filter actually exposes them to the LLM. A wired-up wrapper
/// that's invisible to the orchestrator is dead code.
#[test]
fn orchestrator_lists_memory_tree_tools() {
    let toml = include_str!("../src/openhuman/agent/agents/orchestrator/agent.toml");
    for name in [
        "memory_tree_search_entities",
        "memory_tree_query_topic",
        "memory_tree_query_source",
        "memory_tree_query_global",
        "memory_tree_drill_down",
        "memory_tree_fetch_leaves",
    ] {
        assert!(
            toml.contains(name),
            "orchestrator agent.toml must list '{name}' so the LLM can call it"
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

    // ── Set workspace dir so config_rpc::load_config_with_timeout()
    //    inside the tool resolves to the same workspace we just ingested
    //    into. The tool wrappers always go through that loader (mirrors
    //    the production RPC handlers in retrieval/schemas.rs).
    struct EnvGuard {
        key: &'static str,
        prev: Option<std::ffi::OsString>,
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: see `EnvGuard::set` below — this integration test
            // binary owns the env var for its lifetime.
            unsafe {
                match self.prev.take() {
                    Some(v) => std::env::set_var(self.key, v),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }
    impl EnvGuard {
        fn set(key: &'static str, val: &std::ffi::OsStr) -> Self {
            let prev = std::env::var_os(key);
            // SAFETY: `cargo test` defaults to running each integration
            // test bin in its own process; nothing else in this bin
            // mutates `OPENHUMAN_WORKSPACE`. The guard restores the
            // previous value on drop.
            unsafe { std::env::set_var(key, val) };
            Self { key, prev }
        }
    }
    // Pointing OPENHUMAN_WORKSPACE at `tmp` (not `tmp/workspace`) makes
    // `resolve_config_dir_for_workspace` derive `tmp/workspace` as the
    // resolved workspace_dir — matching what we already passed into
    // `ingest_email` via `cfg.workspace_dir`.
    let _ws_guard = EnvGuard::set("OPENHUMAN_WORKSPACE", tmp.path().as_os_str());

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
