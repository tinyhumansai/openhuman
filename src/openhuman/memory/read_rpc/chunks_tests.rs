//! Sibling tests for the chunk read-RPCs, covering what moved onto the memory
//! contract in #5560.
//!
//! Three things are worth pinning host-side, and none of them is "the driver
//! returned the right rows" — that is the driver's, and its own conformance
//! suite proves it against a real store.
//!
//! 1. **An empty `ChunkQuery` predicate means unfiltered, not match-nothing.**
//!    Every `Vec` on that type defaults to empty because `Default` has to mean
//!    "everything", so a caller that *narrowed* a set down to nothing and sent
//!    it anyway asks for the whole store. Only one field here can narrow, and
//!    the tests below are the difference between an empty page and a leak.
//! 2. **The driver's answer is re-mapped by id, never indexed by position.**
//!    It comes back newest-first and may be shorter than the ids sent, so the
//!    recall response's chunk/score alignment depends on walking the leaves.
//! 3. **A driver with no chunk family degrades to empty rather than erroring.**
//!    These are reads: "this driver stores no chunks" is a true answer to what
//!    they were asked, unlike the destructive handlers in `admin_tests.rs`.

use super::{
    chunk_query_from_filter, chunk_row_from_list_row, content_predicate, list_chunks_rpc,
    list_sources_rpc, pair_leaves_with_rows, search_rpc,
};
use chrono::TimeZone;

use crate::openhuman::config::Config;
use crate::openhuman::memory::api::provider::{ChunkListRow, MemoryProvider};
use crate::openhuman::memory::binding;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;
use tinymemory_api::chunks::{Chunk, Metadata, SourceKind, SourceRef};
use tinymemory_api::null::NullMemoryProvider;

use super::super::types::{ChunkFilter, ChunkRow, PREVIEW_MAX_CHARS};

fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    // Keep any persistence these handlers trigger inside the disposable
    // workspace, exactly as `admin_tests::test_config` does.
    cfg.config_path = tmp.path().join("config.toml");
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    (tmp, cfg)
}

/// Bind the placeholder driver. It implements `MemoryChunks` but does not
/// override `as_chunks`, so the accessor answers `None` — which is the state
/// these degrade tests are about.
fn install_null_driver(cfg: &Config) {
    binding::install_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        Arc::new(NullMemoryProvider::new()) as Arc<dyn MemoryProvider>,
    );
}

fn sample_row(id: &str, content: &str) -> ChunkListRow {
    let timestamp = chrono::Utc
        .timestamp_millis_opt(1_700_000_000_000)
        .single()
        .expect("a valid fixture timestamp");
    let mut metadata = Metadata::point_in_time(SourceKind::Chat, "slack:#eng", "alice", timestamp);
    metadata.tags = vec!["ingested".to_string()];
    metadata.source_ref = Some(SourceRef::new("slack://archives/C123/1"));
    ChunkListRow {
        chunk: Chunk {
            id: id.to_string(),
            content: content.to_string(),
            metadata,
            token_count: 7,
            seq_in_source: 0,
            created_at: timestamp,
            partial_message: false,
        },
        content_path: Some("chat/slack/eng/0.md".to_string()),
        lifecycle_status: Some("admitted".to_string()),
        has_embedding: true,
    }
}

fn row_named(id: &str) -> ChunkRow {
    chunk_row_from_list_row(sample_row(id, "body"))
}

// ── the empty-predicate footgun ──────────────────────────────────────────

/// An absent list and an explicitly empty one both mean "do not filter on
/// this", which is what the SQL did — it skipped the clause for an empty list —
/// and what an empty `Vec` on `ChunkQuery` already means.
#[test]
fn absent_and_empty_filter_lists_are_unfiltered() {
    for filter in [
        ChunkFilter::default(),
        ChunkFilter {
            source_kinds: Some(vec![]),
            source_ids: Some(vec![]),
            entity_ids: Some(vec![]),
            query: Some("   ".into()),
            ..ChunkFilter::default()
        },
    ] {
        let query = chunk_query_from_filter(&filter, 10, 0).expect("an unfiltered query, not None");
        assert!(query.source_kinds.is_empty());
        assert!(query.source_ids.is_empty());
        assert!(query.entity_ids.is_empty());
        assert!(query.ids.is_empty());
        assert_eq!(query.content_contains, None);
    }
}

/// The one field that can narrow to nothing, and the reason this function
/// returns an `Option` at all.
///
/// `source_kind IN ('bogus')` matched no row; an empty `source_kinds` vec would
/// match every row in the store. Those are opposite answers, so the caller has
/// to be told to skip the query rather than handed a query that cannot say it.
#[test]
fn a_wholly_unrecognised_source_kind_set_short_circuits() {
    let filter = ChunkFilter {
        source_kinds: Some(vec!["bogus".into(), "also-bogus".into()]),
        ..ChunkFilter::default()
    };
    assert!(
        chunk_query_from_filter(&filter, 10, 0).is_none(),
        "a set of kinds this build cannot decode matches nothing; sending an empty \
         predicate would return the whole store"
    );
}

/// One unknown value beside a known one is dropped, not fatal — exactly what
/// `source_kind IN ('chat', 'bogus')` did.
#[test]
fn an_unrecognised_source_kind_beside_a_known_one_is_dropped() {
    let filter = ChunkFilter {
        source_kinds: Some(vec!["chat".into(), "bogus".into()]),
        ..ChunkFilter::default()
    };
    let query = chunk_query_from_filter(&filter, 10, 0).expect("the known kind still filters");
    assert_eq!(query.source_kinds, vec![SourceKind::Chat]);
}

/// The page bounds travel on the same value the total is counted from, which is
/// the whole reason the two cannot drift: `count_chunks` drops these and keeps
/// every predicate beside them.
#[test]
fn the_page_bounds_and_predicates_ride_one_query() {
    let filter = ChunkFilter {
        source_ids: Some(vec!["slack:#eng".into()]),
        entity_ids: Some(vec!["email:alice@example.com".into()]),
        since_ms: Some(1_700_000_000_000),
        until_ms: Some(1_700_000_000_500),
        query: Some("phoenix".into()),
        ..ChunkFilter::default()
    };
    let query = chunk_query_from_filter(&filter, 25, 50).expect("a filtered query");
    assert_eq!(query.limit, Some(25));
    assert_eq!(query.offset, Some(50));
    assert_eq!(query.source_ids, vec!["slack:#eng".to_string()]);
    assert_eq!(
        query.entity_ids,
        vec!["email:alice@example.com".to_string()]
    );
    assert_eq!(query.since_ms, Some(1_700_000_000_000));
    assert_eq!(query.until_ms, Some(1_700_000_000_500));
    assert_eq!(query.content_contains.as_deref(), Some("phoenix"));
    // The Memory tab lists dropped rows and renders their status, so the
    // lifecycle predicate the SQL never had must stay off.
    assert!(!query.exclude_dropped);
}

/// The substring travels bare. Wrapping it in `%…%` host-side would double the
/// driver's own wrapping and match nothing.
#[test]
fn the_content_predicate_is_trimmed_and_unwrapped() {
    assert_eq!(content_predicate("  phoenix  ").as_deref(), Some("phoenix"));
    assert_eq!(content_predicate("   "), None);
    assert_eq!(content_predicate(""), None);
    // `%` and `_` are the user's text; escaping them is the driver's job, so
    // they must arrive intact rather than pre-mangled here.
    assert_eq!(content_predicate("50%_off").as_deref(), Some("50%_off"));
}

// ── the wire shape ───────────────────────────────────────────────────────

/// `ChunkListRow` carries no body, and nothing is lost: the preview has always
/// come from the stored `content` column, which is what `Chunk::content` is.
#[test]
fn a_listing_row_renders_every_wire_field() {
    let row = chunk_row_from_list_row(sample_row("chunk-1", "phoenix migration"));
    assert_eq!(row.id, "chunk-1");
    assert_eq!(row.source_kind, "chat");
    assert_eq!(row.source_id, "slack:#eng");
    assert_eq!(row.source_ref.as_deref(), Some("slack://archives/C123/1"));
    assert_eq!(row.owner, "alice");
    assert_eq!(row.timestamp_ms, 1_700_000_000_000);
    assert_eq!(row.token_count, 7);
    assert_eq!(row.lifecycle_status, "admitted");
    assert_eq!(row.content_path.as_deref(), Some("chat/slack/eng/0.md"));
    assert_eq!(row.content_preview.as_deref(), Some("phoenix migration"));
    assert!(row.has_embedding);
    assert_eq!(row.tags, vec!["ingested".to_string()]);
}

/// An empty body is `None`, not `Some("")` — the frontend reads the absence.
#[test]
fn an_empty_body_renders_no_preview() {
    assert_eq!(
        chunk_row_from_list_row(sample_row("c", "")).content_preview,
        None
    );
}

/// The cap is applied in characters, as it always was, and is a no-op on a row
/// whose stored column is already the vault preview.
#[test]
fn the_preview_is_capped_in_characters() {
    let long = "é".repeat(PREVIEW_MAX_CHARS + 10);
    let row = chunk_row_from_list_row(sample_row("c", &long));
    assert_eq!(
        row.content_preview.as_deref().map(|p| p.chars().count()),
        Some(PREVIEW_MAX_CHARS)
    );
}

/// A driver with no lifecycle concept reports `None`; the engine's column is
/// `NOT NULL`, so this is the contract's slack rather than a reachable state —
/// and it renders the same word `read_chunk_row` renders.
#[test]
fn an_unrecorded_lifecycle_renders_as_unknown() {
    let mut source = sample_row("c", "body");
    source.lifecycle_status = None;
    assert_eq!(chunk_row_from_list_row(source).lifecycle_status, "unknown");
}

// ── recall hydration ─────────────────────────────────────────────────────

/// The driver answers newest-first and the leaves are score-ordered, so pairing
/// by position would silently re-order recall and mis-pair every score.
#[test]
fn hydrated_rows_follow_leaf_order_not_driver_order() {
    let leaves = vec![
        ("c-3".to_string(), 0.9_f32),
        ("c-1".to_string(), 0.5_f32),
        ("c-2".to_string(), 0.1_f32),
    ];
    // Deliberately a different order from the leaves, which is what the
    // driver's own `ORDER BY timestamp_ms DESC` produces.
    let by_id: HashMap<String, ChunkRow> = ["c-1", "c-2", "c-3"]
        .into_iter()
        .map(|id| (id.to_string(), row_named(id)))
        .collect();

    let (rows, scores) = pair_leaves_with_rows(&leaves, &by_id);
    assert_eq!(
        rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["c-3", "c-1", "c-2"]
    );
    assert_eq!(scores, vec![0.9_f32, 0.5, 0.1]);
}

/// An id the store does not hold contributes no row, so the answer is shorter
/// than the input — and the scores must shorten with it, not slide.
#[test]
fn an_id_the_store_does_not_hold_drops_its_score_too() {
    let leaves = vec![
        ("missing".to_string(), 0.9_f32),
        ("c-1".to_string(), 0.4_f32),
    ];
    let by_id: HashMap<String, ChunkRow> = HashMap::from([("c-1".to_string(), row_named("c-1"))]);

    let (rows, scores) = pair_leaves_with_rows(&leaves, &by_id);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "c-1");
    assert_eq!(scores, vec![0.4_f32]);
}

/// Two level-1 summaries can both point at one chunk. The per-id loop this
/// replaced emitted it once per leaf, and the deduped `ids` predicate must not
/// change that.
#[test]
fn a_chunk_two_summaries_share_appears_once_per_leaf() {
    let leaves = vec![("c-1".to_string(), 0.9_f32), ("c-1".to_string(), 0.2_f32)];
    let by_id: HashMap<String, ChunkRow> = HashMap::from([("c-1".to_string(), row_named("c-1"))]);

    let (rows, scores) = pair_leaves_with_rows(&leaves, &by_id);
    assert_eq!(rows.len(), 2);
    assert_eq!(scores, vec![0.9_f32, 0.2]);
}

// ── degrading on a driver with no chunk family ───────────────────────────

/// A read, so empty is the honest answer: a driver holding no chunks has none
/// to list and none to count. The total must degrade with the page — a page of
/// nothing labelled "of 431" is worse than either half alone.
#[tokio::test]
async fn list_chunks_reports_empty_without_a_chunks_family() {
    let (_tmp, cfg) = test_config();
    install_null_driver(&cfg);

    let outcome = list_chunks_rpc(&cfg, ChunkFilter::default())
        .await
        .expect("a driver without the chunk family is not an error");
    assert!(outcome.value.chunks.is_empty());
    assert_eq!(outcome.value.total, 0);
    assert!(
        outcome.logs[0].contains("n=0 total=0"),
        "log: {}",
        outcome.logs[0]
    );
}

#[tokio::test]
async fn list_sources_reports_empty_without_a_chunks_family() {
    let (_tmp, cfg) = test_config();
    install_null_driver(&cfg);

    let outcome = list_sources_rpc(&cfg, None)
        .await
        .expect("a driver without the chunk family is not an error");
    assert!(outcome.value.is_empty());
}

#[tokio::test]
async fn search_reports_empty_without_a_chunks_family() {
    let (_tmp, cfg) = test_config();
    install_null_driver(&cfg);

    let outcome = search_rpc(&cfg, "phoenix".into(), 10)
        .await
        .expect("a driver without the chunk family is not an error");
    assert!(outcome.value.is_empty());
    assert!(outcome.logs[0].contains("n=0"), "log: {}", outcome.logs[0]);
}
