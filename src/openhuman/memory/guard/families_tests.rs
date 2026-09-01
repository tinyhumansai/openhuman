//! The wrapped-accessor property — the reason this milestone exists — plus
//! step 2, which lives on `GuardedTree::query_source`.

use crate::openhuman::memory::api::provider::chunks::ChunkQuery;
use crate::openhuman::memory::api::provider::retrieval::{CoverWindowQuery, FastRetrieveQuery};
use crate::openhuman::memory::api::provider::types::SourceScope;
use crate::openhuman::memory::api::provider::{MemoryProvider, MemoryTree};
use crate::openhuman::memory::api::tree::IngestRequest;
use crate::openhuman::memory::api::types::MemoryTaint;

use crate::openhuman::memory::guard::test_support::{
    document, embedded_policy, external_policy, guarded,
};
use crate::openhuman::memory::source_scope::with_source_scope;
use crate::openhuman::security::live_policy;
use crate::openhuman::security::policy::{AutonomyLevel, SecurityPolicy};

fn ingest_request(content: &str) -> IngestRequest {
    IngestRequest {
        namespace: "ns".into(),
        content: content.into(),
        timestamp: None,
        metadata: None,
    }
}

// ── The wrapped-accessor property ───────────────────────────────────────────

#[tokio::test]
async fn guard_as_tree_is_not_the_raw_driver_handle() {
    let (driver, guard) = guarded(embedded_policy());
    let via_guard = guard.as_tree().expect("tree family") as *const dyn MemoryTree;
    let raw = driver.as_tree().expect("tree family") as *const dyn MemoryTree;
    assert!(
        !std::ptr::eq(via_guard, raw),
        "the accessor handed out the driver's own handle — the guard is bypassable"
    );
}

/// The assertion that actually matters. Pointer inequality only proves *some*
/// wrapper exists; this proves the wrapper still enforces.
#[tokio::test]
async fn guard_as_tree_still_applies_policy_reached_through_the_accessor() {
    let dir = std::env::temp_dir();
    let _tier = live_policy::install_scoped(
        std::sync::Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        }),
        dir.clone(),
        dir,
    );

    let (driver, guard) = guarded(embedded_policy());
    let err = guard
        .as_tree()
        .expect("tree family")
        .append(ingest_request("hello"))
        .await
        .expect_err("a readonly tier must refuse a tree write");
    assert!(err.to_string().contains("memory guard: "), "{err}");
    assert_eq!(
        driver.call_count(),
        0,
        "the driver must not be reached at all"
    );
}

#[tokio::test]
async fn every_optional_family_accessor_enforces_the_tier() {
    let dir = std::env::temp_dir();
    let _tier = live_policy::install_scoped(
        std::sync::Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        }),
        dir.clone(),
        dir,
    );
    let (driver, guard) = guarded(embedded_policy());

    // One representative *write* per optional family. Each must be refused
    // before the driver sees it — a family whose decorator forwarded raw would
    // record a call here.
    guard
        .as_ingest()
        .unwrap()
        .ingest_chat(vec![])
        .await
        .expect_err("ingest");
    guard
        .as_documents()
        .unwrap()
        .put_document(document("x", MemoryTaint::Internal))
        .await
        .expect_err("documents");
    guard.as_tree().unwrap().seal("ns").await.expect_err("tree");
    guard
        .as_entities()
        .unwrap()
        .touch_entities("ns", &[])
        .await
        .expect_err("entities");
    guard
        .as_graph()
        .unwrap()
        .kv_put(None, "k", serde_json::Value::Null)
        .await
        .expect_err("graph");
    guard
        .as_diff()
        .unwrap()
        .capture_snapshot("src")
        .await
        .expect_err("diff");
    guard
        .as_goals()
        .unwrap()
        .set_goals(Default::default())
        .await
        .expect_err("goals");
    guard
        .as_tool_memory()
        .unwrap()
        .delete_tool_rule("t", "r")
        .await
        .expect_err("tool_memory");
    guard
        .as_sources()
        .unwrap()
        .forget_source("src")
        .await
        .expect_err("sources");
    guard
        .as_maintenance()
        .unwrap()
        .compact()
        .await
        .expect_err("maintenance");

    assert_eq!(
        driver.call_count(),
        0,
        "at least one family decorator forwarded an unguarded handle: {:?}",
        driver.calls()
    );
}

// ── Step 2 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn guard_fills_query_source_scope_from_the_task_local() {
    let (driver, guard) = guarded(embedded_policy());
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        guard
            .as_tree()
            .unwrap()
            .query_source("ns", "src", 10, None)
            .await
            .expect("query_source");
    })
    .await;
    let call = driver.only_call();
    assert_eq!(call.scoped, Some(true));
    assert_eq!(call.content.as_deref(), Some("slack:#eng"));
}

#[tokio::test]
async fn guard_explicit_scope_is_intersected_with_the_ambient_one() {
    let (driver, guard) = guarded(embedded_policy());
    let explicit = SourceScope::new(["gmail:me"]);
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        guard
            .as_tree()
            .unwrap()
            .query_source("ns", "src", 10, Some(&explicit))
            .await
            .expect("query_source");
    })
    .await;
    assert_eq!(
        driver.only_call().content.as_deref(),
        Some(""),
        "a request outside the ambient allowlist must fail closed"
    );
}

#[tokio::test]
async fn guard_leaves_query_source_unscoped_outside_a_source_scope() {
    let (driver, guard) = guarded(embedded_policy());
    guard
        .as_tree()
        .unwrap()
        .query_source("ns", "src", 10, None)
        .await
        .expect("query_source");
    assert_eq!(driver.only_call().scoped, Some(false));
}

// ── Steps 3 + 4 through a family accessor ───────────────────────────────────

#[tokio::test]
async fn family_writes_are_taint_stamped_too() {
    let (driver, guard) = guarded(embedded_policy());
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        guard
            .as_documents()
            .unwrap()
            .put_document(document("body", MemoryTaint::Internal))
            .await
            .expect("put_document");
    })
    .await;
    assert_eq!(driver.only_call().taint, Some(MemoryTaint::ExternalSync));
}

#[tokio::test]
async fn family_writes_are_not_redacted_for_an_embedded_driver() {
    let secrety = "Authorization: Bearer abcdefghijklmnop";
    let (driver, guard) = guarded(embedded_policy());
    guard
        .as_documents()
        .unwrap()
        .put_document(document(secrety, MemoryTaint::Internal))
        .await
        .expect("put_document");
    assert_eq!(driver.only_call().content.as_deref(), Some(secrety));
}

#[tokio::test]
async fn family_calls_are_refused_for_an_untrusted_external_driver() {
    let (driver, guard) = guarded(external_policy("untrusted"));
    guard
        .as_tree()
        .unwrap()
        .query_source("ns", "src", 10, None)
        .await
        .expect_err("fail-closed");
    assert_eq!(driver.call_count(), 0);
}

// ── Scope narrowing on the chunk and retrieval families ─────────────────────
//
// These mirror `guard_explicit_scope_is_intersected_with_the_ambient_one` for
// the two families added by the module port. They exist because the first
// implementation of both forwarded the caller's scope **unchanged**, which is
// the widening leak `GuardPolicy::narrow_scope` was written to close: a
// source-restricted turn could name a collection outside its restriction and
// have that become the sole query predicate.

#[tokio::test]
async fn chunk_listing_intersects_an_explicit_scope_with_the_ambient_one() {
    let (driver, guard) = guarded(embedded_policy());
    let explicit = SourceScope::new(["gmail:me"]);
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        guard
            .as_chunks()
            .unwrap()
            .list_chunks(&ChunkQuery::default(), Some(&explicit))
            .await
            .expect("list_chunks");
    })
    .await;
    assert_eq!(
        driver.only_call().content.as_deref(),
        Some(""),
        "a chunk query outside the ambient allowlist must fail closed"
    );
}

#[tokio::test]
async fn chunk_listing_inherits_the_ambient_scope_when_none_is_requested() {
    let (driver, guard) = guarded(embedded_policy());
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        guard
            .as_chunks()
            .unwrap()
            .list_chunks(&ChunkQuery::default(), None)
            .await
            .expect("list_chunks");
    })
    .await;
    assert_eq!(
        driver.only_call().content.as_deref(),
        Some("slack:#eng"),
        "the ambient allowlist must reach the driver as a query predicate"
    );
}

#[tokio::test]
async fn fast_retrieve_intersects_an_explicit_scope_with_the_ambient_one() {
    let (driver, guard) = guarded(embedded_policy());
    let explicit = SourceScope::new(["gmail:me"]);
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        guard
            .as_retrieval()
            .unwrap()
            .fast_retrieve(
                "q",
                FastRetrieveQuery {
                    limit: 10,
                    max_hops: 2,
                    time_window_days: None,
                },
                Some(&explicit),
            )
            .await
            .expect("fast_retrieve");
    })
    .await;
    assert_eq!(driver.only_call().content.as_deref(), Some(""));
}

#[tokio::test]
async fn cover_window_intersects_an_explicit_scope_with_the_ambient_one() {
    let (driver, guard) = guarded(embedded_policy());
    let explicit = SourceScope::new(["gmail:me"]);
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        guard
            .as_retrieval()
            .unwrap()
            .cover_window(&CoverWindowQuery::default(), Some(&explicit))
            .await
            .expect("cover_window");
    })
    .await;
    assert_eq!(driver.only_call().content.as_deref(), Some(""));
}

// ── Scope narrowing on the two id-addressed retrieval primitives ────────────
//
// `retrieve_children` and `retrieve_leaves` took no scope argument until the
// review of the module port pointed out what that meant. In-process they were
// still restricted, because the engine reads the ambient task-local — but the
// task-local belongs to the *host's* task and does not cross a bus, so the same
// two methods reached over the module transport were unrestricted. A source
// gate that holds embedded and fails open over a transport is worse than one
// that does neither, because nothing about the call site says which you have.
//
// The scope is an argument now, and these pin that it arrives.

#[tokio::test]
async fn retrieve_children_inherits_the_ambient_scope_when_none_is_requested() {
    let (driver, guard) = guarded(embedded_policy());
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        guard
            .as_retrieval()
            .unwrap()
            .retrieve_children("node", 2, None, None, None)
            .await
            .expect("retrieve_children");
    })
    .await;
    assert_eq!(
        driver.only_call().content.as_deref(),
        Some("slack:#eng"),
        "the ambient allowlist must reach the driver as an explicit argument"
    );
}

#[tokio::test]
async fn retrieve_children_intersects_an_explicit_scope_with_the_ambient_one() {
    let (driver, guard) = guarded(embedded_policy());
    let explicit = SourceScope::new(["gmail:me"]);
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        guard
            .as_retrieval()
            .unwrap()
            .retrieve_children("node", 2, None, None, Some(&explicit))
            .await
            .expect("retrieve_children");
    })
    .await;
    assert_eq!(
        driver.only_call().content.as_deref(),
        Some(""),
        "a walk outside the ambient allowlist must fail closed, not widen"
    );
}

#[tokio::test]
async fn retrieve_leaves_inherits_the_ambient_scope_when_none_is_requested() {
    let (driver, guard) = guarded(embedded_policy());
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        guard
            .as_retrieval()
            .unwrap()
            .retrieve_leaves(&["chunk-1".to_string()], None)
            .await
            .expect("retrieve_leaves");
    })
    .await;
    assert_eq!(
        driver.only_call().content.as_deref(),
        Some("slack:#eng"),
        "naming a chunk id directly must not read around a source restriction"
    );
}

#[tokio::test]
async fn retrieve_leaves_intersects_an_explicit_scope_with_the_ambient_one() {
    let (driver, guard) = guarded(embedded_policy());
    let explicit = SourceScope::new(["gmail:me"]);
    with_source_scope(Some(vec!["slack:#eng".into()]), async {
        guard
            .as_retrieval()
            .unwrap()
            .retrieve_leaves(&["chunk-1".to_string()], Some(&explicit))
            .await
            .expect("retrieve_leaves");
    })
    .await;
    assert_eq!(
        driver.only_call().content.as_deref(),
        Some(""),
        "an explicit scope outside the ambient one must fail closed"
    );
}

// ── Episodic writes are redacted before they cross to a foreign driver ───────
//
// Both members reach a driver that may be a separately compiled module, so the
// content has to be scrubbed on the way out. `insert_event` shipped without
// that scrub once already, and nothing caught it: the recording driver
// discarded the event instead of keeping its text, so no assertion could see
// what the driver was handed. These two tests exist to make that class of
// regression visible, which is why they assert on the recorded content rather
// than merely on the call having happened.

fn episodic_turn(content: &str) -> crate::openhuman::memory::api::provider::EpisodicTurn {
    crate::openhuman::memory::api::provider::EpisodicTurn {
        id: None,
        session_id: "session-1".into(),
        timestamp: 1_700_000_000.0,
        role: "user".into(),
        content: content.into(),
        lesson: None,
        tool_calls_json: None,
        cost_microdollars: 0,
    }
}

fn episodic_event(content: &str) -> crate::openhuman::memory::api::provider::EpisodicEvent {
    crate::openhuman::memory::api::provider::EpisodicEvent {
        event_id: "event-1".into(),
        segment_id: "segment-1".into(),
        session_id: "session-1".into(),
        namespace: "ns".into(),
        kind: crate::openhuman::memory::api::provider::EventKind::Fact,
        content: content.into(),
        subject: None,
        timestamp_ref: None,
        confidence: 1.0,
        embedding: None,
        source_turn_ids: None,
        created_at: 1_700_000_000.0,
    }
}

#[tokio::test]
async fn episodic_writes_are_redacted_for_a_foreign_driver() {
    let secrety = "Authorization: Bearer abcdefghijklmnop";

    let (driver, guard) = guarded(external_policy("trusted"));
    guard
        .as_episodic()
        .expect("episodic family")
        .insert_turn(&episodic_turn(secrety))
        .await
        .expect("insert_turn");
    let turn = driver.only_call();
    assert_ne!(
        turn.content.as_deref(),
        Some(secrety),
        "the bearer token reached a foreign driver verbatim"
    );

    let (driver, guard) = guarded(external_policy("trusted"));
    guard
        .as_episodic()
        .expect("episodic family")
        .insert_event(&episodic_event(secrety))
        .await
        .expect("insert_event");
    let event = driver.only_call();
    assert_ne!(
        event.content.as_deref(),
        Some(secrety),
        "the bearer token reached a foreign driver verbatim"
    );
}

/// The other half, so the test above cannot pass by redacting everything
/// everywhere: an in-process driver is the same address space, so scrubbing
/// there would cost fidelity for no privacy gain.
#[tokio::test]
async fn episodic_writes_are_not_redacted_for_an_embedded_driver() {
    let secrety = "Authorization: Bearer abcdefghijklmnop";

    let (driver, guard) = guarded(embedded_policy());
    guard
        .as_episodic()
        .expect("episodic family")
        .insert_event(&episodic_event(secrety))
        .await
        .expect("insert_event");
    assert_eq!(driver.only_call().content.as_deref(), Some(secrety));
}

// ── The defaulted doors ─────────────────────────────────────────────────────
//
// Every method a family trait *defaults* is a method a decorator can silently
// fail to forward: the impl compiles, and the default answers `Unsupported`
// however capable the driver below is. `RecordingProvider` overrides the five
// #5560 doors and `diagnose`, so a decorator that dropped one is the only way
// these can come back unsupported.

/// One summary context, so the fold tests read as the two things they differ
/// in rather than as seven repeated budget fields.
fn summary_context() -> crate::openhuman::memory::api::provider::content::SummaryContext {
    crate::openhuman::memory::api::provider::content::SummaryContext {
        tree_id: "segment-1".into(),
        tree_kind: "source".into(),
        target_level: 0,
        token_budget: 2_000,
        input_token_budget: 50_000,
        overhead_reserve_tokens: 2_048,
        ask: None,
    }
}

fn summary_input(content: &str) -> crate::openhuman::memory::api::provider::content::SummaryInput {
    let at = chrono::Utc::now();
    crate::openhuman::memory::api::provider::content::SummaryInput {
        id: "turn-1".into(),
        content: content.into(),
        token_count: 4,
        entities: Vec::new(),
        topics: Vec::new(),
        time_range_start: at,
        time_range_end: at,
        score: 0.5,
    }
}

#[tokio::test]
async fn the_defaulted_doors_are_forwarded_rather_than_refused() {
    let (driver, guard) = guarded(embedded_policy());

    guard
        .as_tree()
        .expect("tree family")
        .summarise(&[summary_input("hello")], &summary_context())
        .await
        .expect("summarise must reach the driver, not the trait default");
    guard
        .as_tree()
        .expect("tree family")
        .root_summaries_with_caps(8_000, 32_000)
        .await
        .expect("root_summaries_with_caps must reach the driver");
    guard
        .as_chunks()
        .expect("chunks family")
        .chunk_score("chunk-1")
        .await
        .expect("chunk_score must reach the driver");
    guard
        .as_chunks()
        .expect("chunks family")
        .source_ingest_status(&[])
        .await
        .expect("source_ingest_status must reach the driver");
    guard
        .as_maintenance()
        .expect("maintenance family")
        .degraded_state()
        .await
        .expect("degraded_state must reach the driver");
    guard
        .as_maintenance()
        .expect("maintenance family")
        .diagnose()
        .await
        .expect("diagnose must reach the driver");

    // The seven the runtime-tree round added (contract 4.0). Six of them carry
    // the `tree_summarizer_*` RPC surface and the `tree-summarizer` CLI; the
    // seventh is what `memory_flavour` reads. A `GuardedTree` that forgot one
    // would refuse a driver that serves it perfectly well — the shape of the
    // `diagnose` bug this test was written for.
    let tree = guard.as_tree().expect("tree family");
    let at = chrono::Utc::now();
    tree.runtime_buffer_write("team", "hello", at, None)
        .await
        .expect("runtime_buffer_write must reach the driver");
    tree.runtime_read_node("team", "root")
        .await
        .expect("runtime_read_node must reach the driver");
    tree.runtime_read_children("team", "root")
        .await
        .expect("runtime_read_children must reach the driver");
    tree.runtime_tree_status("team")
        .await
        .expect("runtime_tree_status must reach the driver");
    tree.runtime_summarize("team", at)
        .await
        .expect("runtime_summarize must reach the driver");
    tree.runtime_rebuild("team")
        .await
        .expect("runtime_rebuild must reach the driver");
    tree.flavour_profile("persona/communication")
        .await
        .expect("flavour_profile must reach the driver");

    let methods: Vec<String> = driver.calls().into_iter().map(|call| call.method).collect();
    assert_eq!(
        methods,
        vec![
            "tree.summarise",
            "tree.root_summaries_with_caps",
            "chunks.chunk_score",
            "chunks.source_ingest_status",
            "maintenance.degraded_state",
            "maintenance.diagnose",
            "tree.runtime_buffer_write",
            "tree.runtime_read_node",
            "tree.runtime_read_children",
            "tree.runtime_tree_status",
            "tree.runtime_summarize",
            "tree.runtime_rebuild",
            "tree.flavour_profile",
        ]
    );
}

/// `summarise` is the only member of the tree family besides `append` that
/// hands prose *out* of the host — to the driver's own chat provider — so it
/// takes `append`'s outbound scrub.
#[tokio::test]
async fn summarise_inputs_are_redacted_for_a_foreign_driver() {
    let secrety = "Authorization: Bearer abcdefghijklmnop";

    let (driver, guard) = guarded(external_policy("trusted"));
    guard
        .as_tree()
        .expect("tree family")
        .summarise(&[summary_input(secrety)], &summary_context())
        .await
        .expect("summarise");
    assert_ne!(
        driver.only_call().content.as_deref(),
        Some(secrety),
        "the bearer token reached a foreign driver's summariser verbatim"
    );
}

/// The other half: an embedded driver is the same address space, so scrubbing
/// there would cost the fold fidelity for no privacy gain — and the borrowed
/// slice must be forwarded untouched rather than re-owned.
#[tokio::test]
async fn summarise_inputs_are_not_redacted_for_an_embedded_driver() {
    let secrety = "Authorization: Bearer abcdefghijklmnop";

    let (driver, guard) = guarded(embedded_policy());
    guard
        .as_tree()
        .expect("tree family")
        .summarise(&[summary_input(secrety)], &summary_context())
        .await
        .expect("summarise");
    assert_eq!(driver.only_call().content.as_deref(), Some(secrety));
}
