//! Golden-workspace fixture: seeding, read-back, and schema-manifest capture.
//!
//! This module is the engine behind `tests/memory_golden_fixture_e2e.rs`, the
//! schema gate that stands between a memory-store change and a corrupted user
//! workspace. It lives in-crate rather than in the test file because seeding a
//! *complete* workspace needs `pub(crate)` reach that an integration test does
//! not have — `MemoryClient::profile_conn`, `trees::store::insert_summary_tx`,
//! and `trees::store::update_tree_after_seal_tx` are all deliberately
//! crate-private escape hatches.
//!
//! # The four entry points
//!
//! - [`seed`] materialises every structure the gate protects into a workspace,
//!   using production write paths (`memory::ops::*` and the same typed store
//!   helpers the archivist and the learning cache call).
//! - [`read_back`] reads all of it out again through `memory::ops` — proving
//!   the *code path* still works, not merely that the schema still parses.
//! - [`init_fresh_schema`] stands up an empty workspace's schema, which is the
//!   only way to see an *in-place* DDL redefinition (`CREATE … IF NOT EXISTS`
//!   is a no-op against a DB that already holds the name).
//! - [`schema_manifest`] dumps `sqlite_master` (tables, indexes, triggers) plus
//!   `PRAGMA user_version` across every `*.db` in the workspace, normalised to
//!   a deterministic, diffable text form.
//!
//! # Why the fixture must be captured, not synthesised
//!
//! The committed fixture under `tests/fixtures/memory_golden/` was produced by
//! a **specific past build**. The manifest is derived from that fixture by
//! [`schema_manifest`], never hand-written. That combination is what makes the
//! gate bite: editing a `CREATE TABLE` in `namespace_store/init.rs` *and*
//! editing the manifest to match still fails, because the committed `.db` was
//! built by the older binary and no longer matches the new DDL. Making the
//! suite green requires deliberately regenerating the fixture — a visible,
//! reviewable act. See `tests/fixtures/memory_golden/README.md`.
//!
//! Debug logging uses the `[golden]` prefix throughout. Nothing seeded here is
//! real user data: every value is a fixed literal chosen to be obviously
//! synthetic.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use chrono::{DateTime, TimeZone, Utc};

use crate::openhuman::config::Config;
use crate::openhuman::memory::ops::{
    doc_list, doc_put, graph_query, graph_upsert, kv_get, memory_query_namespace, GraphQueryParams,
    GraphUpsertParams, KvGetDeleteParams, KvSetParams, NamespaceOnlyParams, PutDocParams,
};
use crate::openhuman::memory::rpc_models::QueryNamespaceRequest;
use crate::openhuman::memory::store::chunks;
use crate::openhuman::memory::store::chunks::types::{Chunk, Metadata, SourceKind, SourceRef};
use crate::openhuman::memory::store::namespace_store::{events, fts5, profile, segments};
use crate::openhuman::memory::store::trees;
use crate::openhuman::memory::store::trees::types::{SummaryNode, Tree, TreeKind, TreeStatus};

// ── Fixture identity ─────────────────────────────────────────────────────────
//
// Every constant below is part of the fixture's contract: the committed `.db`
// contains rows under exactly these keys, and `read_back` looks them up by
// name. Changing one means regenerating the fixture.

/// First seeded namespace.
pub const NAMESPACE_PRIMARY: &str = "golden-primary";
/// Second seeded namespace — the gate needs ≥ 2 so namespace scoping is real.
pub const NAMESPACE_SECONDARY: &str = "golden-secondary";
/// Document key in [`NAMESPACE_PRIMARY`].
pub const DOC_KEY_PRIMARY: &str = "golden-doc-primary";
/// Document key in [`NAMESPACE_SECONDARY`].
pub const DOC_KEY_SECONDARY: &str = "golden-doc-secondary";
/// Body of the primary document; also the target of [`RECALL_QUERY`].
pub const DOC_CONTENT_PRIMARY: &str =
    "The golden fixture pins the memory workspace schema for regression testing.";
/// Body of the secondary document.
pub const DOC_CONTENT_SECONDARY: &str =
    "A second namespace exists so namespace scoping is exercised, not assumed.";
/// Key used for both the global and the namespace-scoped KV write.
pub const KV_KEY: &str = "golden-kv-canary";
/// Graph triple subject.
pub const GRAPH_SUBJECT: &str = "golden-subject";
/// Graph triple predicate.
pub const GRAPH_PREDICATE: &str = "relates-to";
/// Graph triple object.
pub const GRAPH_OBJECT: &str = "golden-object";
/// Session id shared by the episodic row, the segment, and the event.
pub const SESSION_ID: &str = "golden-session";
/// Seeded conversation segment id.
pub const SEGMENT_ID: &str = "golden-segment";
/// Seeded event id.
pub const EVENT_ID: &str = "golden-event";
/// Seeded profile facet key.
pub const PROFILE_KEY: &str = "golden/verbosity";
/// Seeded profile facet value.
pub const PROFILE_VALUE: &str = "concise";
/// Seeded summary-tree id.
pub const TREE_ID: &str = "golden-tree";
/// Seeded summary node id (the sealed root of [`TREE_ID`]).
pub const SUMMARY_ID: &str = "golden-summary";
/// Embedding model signature stamped on every seeded vector.
pub const MODEL_SIGNATURE: &str = "golden-fixture/dim-4";
/// The deterministic vector written to every embedding tier.
pub const EMBEDDING: [f32; 4] = [0.25, 0.5, 0.75, 1.0];
/// Fixed recall query — [`read_back`] asserts its result set exactly.
pub const RECALL_QUERY: &str = "golden fixture schema";

/// Fixed timestamp for every seeded row, so a regenerated fixture differs from
/// the committed one only where the *schema* differs.
fn fixed_time() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0)
        .single()
        .expect("fixed fixture timestamp is valid")
}

fn fixed_epoch_secs() -> f64 {
    1_700_000_000.0
}

/// Build a [`Config`] rooted at `workspace`, for the tinycortex-backed tiers
/// (`chunks::*` / `trees::*`) which resolve their DB path from `workspace_dir`.
fn fixture_config(workspace: &Path) -> Config {
    let mut config = Config::default();
    config.workspace_dir = workspace.to_path_buf();
    config
}

// ── Seeding ──────────────────────────────────────────────────────────────────

/// Seed a complete golden workspace at `workspace`.
///
/// The caller must have bound the process-global memory client to `workspace`
/// (`memory::global::init`) and pointed `OPENHUMAN_WORKSPACE` at it first, so
/// the `memory::ops` write paths land in the same place as the direct store
/// writes below.
///
/// Idempotent: every write is an upsert or `INSERT OR REPLACE`, so re-seeding
/// an already-seeded workspace is a no-op at the row level.
pub async fn seed(workspace: &Path) -> Result<()> {
    tracing::debug!(workspace = %workspace.display(), "[golden] seeding golden workspace");

    seed_documents().await?;
    seed_kv().await?;
    seed_graph().await?;

    let client = crate::openhuman::memory::global::client()
        .map_err(|e| anyhow::anyhow!("[golden] memory client not bound: {e}"))?;
    let conn = client.profile_conn();

    seed_episodic(&conn)?;
    seed_segment(&conn)?;
    seed_event(&conn)?;
    seed_profile(&conn)?;
    drop(conn);

    seed_chunk_and_tree(workspace)?;

    tracing::debug!("[golden] seeding complete");
    Ok(())
}

async fn seed_documents() -> Result<()> {
    for (namespace, key, content) in [
        (NAMESPACE_PRIMARY, DOC_KEY_PRIMARY, DOC_CONTENT_PRIMARY),
        (
            NAMESPACE_SECONDARY,
            DOC_KEY_SECONDARY,
            DOC_CONTENT_SECONDARY,
        ),
    ] {
        tracing::debug!(namespace, key, "[golden] seeding document");
        doc_put(PutDocParams {
            namespace: namespace.to_string(),
            key: key.to_string(),
            title: format!("Golden fixture document ({namespace})"),
            content: content.to_string(),
            source_type: "doc".to_string(),
            priority: "medium".to_string(),
            tags: vec!["golden".to_string()],
            metadata: serde_json::json!({ "fixture": true }),
            category: "core".to_string(),
            session_id: None,
            document_id: None,
        })
        .await
        .map_err(|e| anyhow::anyhow!("[golden] doc_put({namespace}/{key}) failed: {e}"))?;
    }
    Ok(())
}

async fn seed_kv() -> Result<()> {
    for namespace in [None, Some(NAMESPACE_PRIMARY.to_string())] {
        tracing::debug!(?namespace, key = KV_KEY, "[golden] seeding kv");
        crate::openhuman::memory::ops::kv_set(KvSetParams {
            namespace: namespace.clone(),
            key: KV_KEY.to_string(),
            value: serde_json::json!({ "fixture": "golden", "v": 1 }),
        })
        .await
        .map_err(|e| anyhow::anyhow!("[golden] kv_set({namespace:?}) failed: {e}"))?;
    }
    Ok(())
}

async fn seed_graph() -> Result<()> {
    tracing::debug!(subject = GRAPH_SUBJECT, "[golden] seeding graph triple");
    graph_upsert(GraphUpsertParams {
        namespace: Some(NAMESPACE_PRIMARY.to_string()),
        subject: GRAPH_SUBJECT.to_string(),
        predicate: GRAPH_PREDICATE.to_string(),
        object: GRAPH_OBJECT.to_string(),
        attrs: serde_json::json!({ "fixture": true }),
    })
    .await
    .map_err(|e| anyhow::anyhow!("[golden] graph_upsert failed: {e}"))?;
    Ok(())
}

type SharedConn = std::sync::Arc<parking_lot::Mutex<rusqlite::Connection>>;

/// Episodic row — also materialises the `episodic_fts` shadow tables through
/// the `episodic_ai` trigger.
fn seed_episodic(conn: &SharedConn) -> Result<()> {
    tracing::debug!(session = SESSION_ID, "[golden] seeding episodic row");
    fts5::episodic_insert(
        conn,
        &fts5::EpisodicEntry {
            id: None,
            session_id: SESSION_ID.to_string(),
            timestamp: fixed_epoch_secs(),
            role: "user".to_string(),
            content: "Golden fixture episodic turn about the memory schema.".to_string(),
            lesson: Some("Fixtures beat hand-written constants.".to_string()),
            tool_calls_json: None,
            cost_microdollars: 0,
        },
    )
    .context("[golden] episodic_insert")
}

/// A sealed (summarised) conversation segment with both embedding tiers.
fn seed_segment(conn: &SharedConn) -> Result<()> {
    tracing::debug!(
        segment = SEGMENT_ID,
        "[golden] seeding conversation segment"
    );
    let now = fixed_epoch_secs();
    segments::segment_create(
        conn,
        SEGMENT_ID,
        SESSION_ID,
        NAMESPACE_PRIMARY,
        1,
        Some(0),
        now,
        now,
    )
    .context("[golden] segment_create")?;
    segments::segment_append_turn(conn, SEGMENT_ID, 1, Some(1), now, now)
        .context("[golden] segment_append_turn")?;
    segments::segment_close(conn, SEGMENT_ID, now).context("[golden] segment_close")?;
    segments::segment_set_summary(conn, SEGMENT_ID, "Golden fixture segment summary.", now)
        .context("[golden] segment_set_summary")?;
    segments::segment_set_embedding(conn, SEGMENT_ID, &EMBEDDING, now)
        .context("[golden] segment_set_embedding")?;
    segments::segment_embedding_upsert(conn, SEGMENT_ID, MODEL_SIGNATURE, &EMBEDDING, now)
        .context("[golden] segment_embedding_upsert")
}

/// An event row (materialising the `event_fts` shadow tables via trigger) plus
/// its per-model embedding.
fn seed_event(conn: &SharedConn) -> Result<()> {
    tracing::debug!(event = EVENT_ID, "[golden] seeding event row");
    let now = fixed_epoch_secs();
    events::event_insert(
        conn,
        &events::EventRecord {
            event_id: EVENT_ID.to_string(),
            segment_id: SEGMENT_ID.to_string(),
            session_id: SESSION_ID.to_string(),
            namespace: NAMESPACE_PRIMARY.to_string(),
            event_type: events::EventType::Decision,
            content: "Decided to pin the memory schema with a captured fixture.".to_string(),
            subject: Some(GRAPH_SUBJECT.to_string()),
            timestamp_ref: None,
            confidence: 0.9,
            embedding: Some(EMBEDDING.to_vec()),
            source_turn_ids: None,
            created_at: now,
        },
    )
    .context("[golden] event_insert")?;
    events::event_embedding_upsert(conn, EVENT_ID, MODEL_SIGNATURE, &EMBEDDING, now)
        .context("[golden] event_embedding_upsert")
}

/// A `user_profile` facet — the learning tier.
fn seed_profile(conn: &SharedConn) -> Result<()> {
    tracing::debug!(key = PROFILE_KEY, "[golden] seeding profile facet");
    profile::profile_upsert(
        conn,
        "golden-facet",
        &profile::FacetType::Preference,
        PROFILE_KEY,
        PROFILE_VALUE,
        0.8,
        Some(SEGMENT_ID),
        fixed_epoch_secs(),
    )
    .context("[golden] profile_upsert")
}

/// The tinycortex substrate: one leaf chunk with an embedding, plus a tree
/// sealed to an L1 summary node with its own embedding.
fn seed_chunk_and_tree(workspace: &Path) -> Result<()> {
    let config = fixture_config(workspace);
    let at = fixed_time();

    let metadata = Metadata {
        source_kind: SourceKind::Document,
        source_id: "golden-source".to_string(),
        owner: "golden-owner".to_string(),
        timestamp: at,
        time_range: (at, at),
        tags: vec!["golden".to_string()],
        source_ref: Some(SourceRef::new("golden://fixture/1")),
        path_scope: Some("golden".to_string()),
    };
    let chunk = Chunk {
        id: chunks::types::chunk_id(
            SourceKind::Document,
            "golden-source",
            0,
            DOC_CONTENT_PRIMARY,
        ),
        content: DOC_CONTENT_PRIMARY.to_string(),
        metadata,
        token_count: 20,
        seq_in_source: 0,
        created_at: at,
        partial_message: false,
    };
    let chunk_id = chunk.id.clone();
    tracing::debug!(chunk = %chunk_id, "[golden] seeding tinycortex leaf chunk");
    chunks::store::upsert_chunks(&config, std::slice::from_ref(&chunk))
        .context("[golden] upsert_chunks")?;
    chunks::store::set_chunk_embedding(&config, &chunk_id, &EMBEDDING)
        .context("[golden] set_chunk_embedding")?;

    tracing::debug!(tree = TREE_ID, "[golden] seeding summary tree");
    trees::store::insert_tree(
        &config,
        &Tree {
            id: TREE_ID.to_string(),
            kind: TreeKind::Source,
            scope: "golden-source".to_string(),
            root_id: None,
            max_level: 0,
            status: TreeStatus::Active,
            created_at: at,
            last_sealed_at: None,
            ask: None,
        },
    )
    .context("[golden] insert_tree")?;

    let node = SummaryNode {
        id: SUMMARY_ID.to_string(),
        tree_id: TREE_ID.to_string(),
        tree_kind: TreeKind::Source,
        level: 1,
        parent_id: None,
        child_ids: vec![chunk_id.clone()],
        content: "Golden fixture summary node.".to_string(),
        token_count: 8,
        entities: vec![GRAPH_SUBJECT.to_string()],
        topics: vec!["golden".to_string()],
        time_range_start: at,
        time_range_end: at,
        score: 1.0,
        sealed_at: at,
        deleted: false,
        embedding: None,
        doc_id: None,
        version_ms: None,
    };

    // Seal in one transaction, exactly as the production seal path does.
    chunks::store::with_connection(&config, |conn| {
        let tx = conn.unchecked_transaction()?;
        trees::store::insert_summary_tx(&tx, &node, None, MODEL_SIGNATURE)?;
        trees::store::update_tree_after_seal_tx(&tx, TREE_ID, SUMMARY_ID, 1, at)?;
        tx.commit()?;
        Ok(())
    })
    .context("[golden] seal summary tree")?;

    trees::store::set_summary_embedding(&config, SUMMARY_ID, &EMBEDDING)
        .context("[golden] set_summary_embedding")?;
    Ok(())
}

/// Materialise a **fresh** workspace's schema at `workspace` — no rows, no
/// process-global memory client, just the bootstrap DDL both tiers run on
/// every open.
///
/// This exists to close a blind spot in the "reopen the committed fixture"
/// check. `CREATE TABLE / INDEX / TRIGGER IF NOT EXISTS` is a **no-op** against
/// a database that already has the name, so redefining an existing object
/// in place is invisible when the gate only ever reopens an old DB. A fresh
/// DB takes the new DDL, so comparing it to the same manifest catches the edit.
pub async fn init_fresh_schema(workspace: &Path) -> Result<()> {
    tracing::debug!(workspace = %workspace.display(), "[golden] initialising a fresh schema");
    std::fs::create_dir_all(workspace).context("[golden] create fresh workspace dir")?;

    // Host unified tier.
    let memory = crate::openhuman::memory::store::UnifiedMemory::new(
        workspace,
        std::sync::Arc::new(tinymemory_api::host::NoopEmbedding),
        None,
    )
    .context("[golden] UnifiedMemory::new on a fresh workspace")?;

    // The crate KV tier (`kv_global` / `kv_namespace` + `idx_kv_ns`) is created
    // **lazily** by `KvStore::from_shared_connection` on first use, not by
    // `UnifiedMemory::new`. Touch it, or the fresh schema is missing `idx_kv_ns`
    // and the gate reports a false drift.
    memory
        .kv_get_global("golden-schema-probe")
        .await
        .map_err(|e| anyhow::anyhow!("[golden] crate KV tier init: {e}"))?;

    // tinycortex chunk-DB substrate.
    let config = fixture_config(workspace);
    chunks::store::with_connection(&config, |_conn| Ok(()))
        .context("[golden] tinycortex chunk-DB init on a fresh workspace")?;
    Ok(())
}

// ── Read-back ────────────────────────────────────────────────────────────────

/// Everything [`read_back`] recovered from a seeded workspace.
///
/// Deliberately plain data so the test can assert on it without re-deriving
/// any of the lookup logic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Readback {
    /// Document keys found in [`NAMESPACE_PRIMARY`], sorted.
    pub primary_doc_keys: Vec<String>,
    /// Document keys found in [`NAMESPACE_SECONDARY`], sorted.
    pub secondary_doc_keys: Vec<String>,
    /// Whether the global-scope KV value round-tripped.
    pub kv_global_present: bool,
    /// Whether the namespace-scope KV value round-tripped.
    pub kv_namespace_present: bool,
    /// Number of graph triples matching the seeded subject.
    pub graph_hits: usize,
    /// Session ids of episodic rows recovered for [`SESSION_ID`].
    pub episodic_sessions: Vec<String>,
    /// Segment ids recovered for [`NAMESPACE_PRIMARY`], sorted.
    pub segment_ids: Vec<String>,
    /// Event ids recovered for the seeded segment, sorted.
    pub event_ids: Vec<String>,
    /// Profile facet keys recovered, sorted.
    pub profile_keys: Vec<String>,
    /// Leaf chunk ids present in the tinycortex substrate, sorted.
    pub chunk_ids: Vec<String>,
    /// Summary node ids present under [`TREE_ID`], sorted.
    pub summary_ids: Vec<String>,
    /// Whether the seeded tree reports a sealed root.
    pub tree_sealed: bool,
    /// Whether every embedding tier read back the exact seeded vector.
    pub embeddings_match: bool,
    /// Chunk contents returned by the fixed [`RECALL_QUERY`], sorted.
    pub recall_chunks: Vec<String>,
}

/// Read every seeded structure back out of `workspace`.
///
/// Documents, KV and graph go through `memory::ops` — the same handlers the
/// JSON-RPC surface calls — so this proves the *code path*, not just that the
/// schema parses. The episodic / segment / event / profile / substrate tiers
/// have no `ops` reader, so they use the same typed store helpers their
/// production readers use.
pub async fn read_back(workspace: &Path) -> Result<Readback> {
    tracing::debug!(workspace = %workspace.display(), "[golden] reading golden workspace back");

    let primary_doc_keys = doc_keys_in(NAMESPACE_PRIMARY).await?;
    let secondary_doc_keys = doc_keys_in(NAMESPACE_SECONDARY).await?;

    let kv_global_present = kv_get(KvGetDeleteParams {
        namespace: None,
        key: KV_KEY.to_string(),
    })
    .await
    .map_err(|e| anyhow::anyhow!("[golden] kv_get(global) failed: {e}"))?
    .value
    .is_some();
    let kv_namespace_present = kv_get(KvGetDeleteParams {
        namespace: Some(NAMESPACE_PRIMARY.to_string()),
        key: KV_KEY.to_string(),
    })
    .await
    .map_err(|e| anyhow::anyhow!("[golden] kv_get(namespace) failed: {e}"))?
    .value
    .is_some();

    let graph_hits = graph_query(GraphQueryParams {
        namespace: Some(NAMESPACE_PRIMARY.to_string()),
        subject: Some(GRAPH_SUBJECT.to_string()),
        predicate: None,
    })
    .await
    .map_err(|e| anyhow::anyhow!("[golden] graph_query failed: {e}"))?
    .value
    .len();

    let client = crate::openhuman::memory::global::client()
        .map_err(|e| anyhow::anyhow!("[golden] memory client not bound: {e}"))?;
    let conn = client.profile_conn();

    let episodic_sessions: Vec<String> = fts5::episodic_session_entries(&conn, SESSION_ID)
        .context("[golden] episodic_session_entries")?
        .into_iter()
        .map(|entry| entry.session_id)
        .collect();

    let mut segment_ids: Vec<String> =
        segments::segments_by_namespace(&conn, NAMESPACE_PRIMARY, 16)
            .context("[golden] segments_by_namespace")?
            .into_iter()
            .map(|segment| segment.segment_id)
            .collect();
    segment_ids.sort();

    let mut event_ids: Vec<String> = events::events_for_segment(&conn, SEGMENT_ID)
        .context("[golden] events_for_segment")?
        .into_iter()
        .map(|event| event.event_id)
        .collect();
    event_ids.sort();

    let mut profile_keys: Vec<String> = profile::profile_select_all(&conn)
        .context("[golden] profile_select_all")?
        .into_iter()
        .map(|facet| facet.key)
        .collect();
    profile_keys.sort();

    let segment_vector = segments::segment_embedding_get(&conn, SEGMENT_ID, MODEL_SIGNATURE)
        .context("[golden] segment_embedding_get")?;
    let event_vector = events::event_embedding_get(&conn, EVENT_ID, MODEL_SIGNATURE)
        .context("[golden] event_embedding_get")?;
    drop(conn);

    let config = fixture_config(workspace);
    let mut chunk_ids: Vec<String> = chunks::store::list_chunks(
        &config,
        &chunks::ListChunksQuery {
            limit: Some(64),
            ..Default::default()
        },
    )
    .context("[golden] list_chunks")?
    .into_iter()
    .map(|chunk| chunk.id)
    .collect();
    chunk_ids.sort();

    let mut summary_ids: Vec<String> = trees::store::list_summaries_at_level(&config, TREE_ID, 1)
        .context("[golden] list_summaries_at_level")?
        .into_iter()
        .map(|node| node.id)
        .collect();
    summary_ids.sort();

    let tree_sealed = trees::store::get_tree(&config, TREE_ID)
        .context("[golden] get_tree")?
        .is_some_and(|tree| tree.root_id.as_deref() == Some(SUMMARY_ID));

    let chunk_vector = chunk_ids
        .first()
        .map(|id| chunks::store::get_chunk_embedding(&config, id))
        .transpose()
        .context("[golden] get_chunk_embedding")?
        .flatten();
    let summary_vector = trees::store::get_summary_embedding(&config, SUMMARY_ID)
        .context("[golden] get_summary_embedding")?;

    let embeddings_match = [segment_vector, event_vector, chunk_vector, summary_vector]
        .iter()
        .all(|vector| vector.as_deref() == Some(&EMBEDDING[..]));

    // Fixed-query recall through the production handler. Asserting on chunk
    // *contents* rather than scores keeps this deterministic across embedding
    // backends while still proving the retrieval path runs end to end.
    let recall_envelope = memory_query_namespace(QueryNamespaceRequest {
        namespace: NAMESPACE_PRIMARY.to_string(),
        query: RECALL_QUERY.to_string(),
        include_references: Some(true),
        document_ids: None,
        limit: Some(16),
        max_chunks: None,
    })
    .await
    .map_err(|e| anyhow::anyhow!("[golden] memory_query_namespace failed: {e}"))?
    .value;
    anyhow::ensure!(
        recall_envelope.error.is_none(),
        "[golden] recall returned an error envelope: {:?}",
        recall_envelope.error
    );
    let mut recall_chunks: Vec<String> = recall_envelope
        .data
        .and_then(|response| response.context)
        .map(|context| {
            context
                .chunks
                .into_iter()
                .map(|chunk| chunk.content)
                .collect()
        })
        .unwrap_or_default();
    recall_chunks.sort();

    let readback = Readback {
        primary_doc_keys,
        secondary_doc_keys,
        kv_global_present,
        kv_namespace_present,
        graph_hits,
        episodic_sessions,
        segment_ids,
        event_ids,
        profile_keys,
        chunk_ids,
        summary_ids,
        tree_sealed,
        embeddings_match,
        recall_chunks,
    };
    tracing::debug!(?readback, "[golden] read-back complete");
    Ok(readback)
}

async fn doc_keys_in(namespace: &str) -> Result<Vec<String>> {
    let listed = doc_list(Some(NamespaceOnlyParams {
        namespace: namespace.to_string(),
    }))
    .await
    .map_err(|e| anyhow::anyhow!("[golden] doc_list({namespace}) failed: {e}"))?;
    // Strict on shape. A tolerant `unwrap_or_default()` here would turn a
    // change to the `doc_list` envelope into "zero documents", which reads as
    // a data-loss failure and hides the real cause.
    let rows = listed
        .value
        .get("documents")
        .and_then(|v| v.as_array())
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "[golden] doc_list({namespace}) envelope has no `documents` array: {}",
                listed.value
            )
        })?;
    let mut keys: Vec<String> = Vec::with_capacity(rows.len());
    for row in rows {
        let key = row
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("[golden] doc_list row has no `key`: {row}"))?;
        keys.push(key.to_string());
    }
    keys.sort();
    Ok(keys)
}

// ── Schema manifest ──────────────────────────────────────────────────────────

/// Recursively collect every `*.db` under `dir`, sorted by path.
pub fn db_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("db") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

/// Collapse every whitespace run in a DDL statement to a single space.
///
/// SQLite stores `sqlite_master.sql` verbatim, so re-indenting a `CREATE TABLE`
/// would otherwise read as a schema change. Formatting is not the contract;
/// structure is.
fn normalize_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Deterministic, diffable dump of every schema object in `workspace`.
///
/// One line per object, of the form:
///
/// ```text
/// <db-relative-path>\t<type>\t<name>\t<normalized sql>
/// ```
///
/// plus one `pragma\tuser_version` line per DB file. Lines are collected into a
/// `BTreeSet`, so the result is order-independent and compares as a **set** —
/// the test reports missing and extra objects separately rather than a
/// whole-file diff.
///
/// Covers `type IN ('table','index','trigger')`, including SQLite's internal
/// `sqlite_autoindex_*` entries (deterministic consequences of the DDL) and the
/// FTS5 shadow tables.
pub fn schema_manifest(workspace: &Path) -> Result<BTreeSet<String>> {
    let mut lines = BTreeSet::new();
    let files = db_files(workspace);
    anyhow::ensure!(
        !files.is_empty(),
        "[golden] no *.db files found under {}",
        workspace.display()
    );

    for db in files {
        let relative = db
            .strip_prefix(workspace)
            .unwrap_or(&db)
            .to_string_lossy()
            .replace('\\', "/");
        tracing::debug!(db = %relative, "[golden] dumping schema");

        let conn =
            rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .with_context(|| format!("[golden] open {relative} read-only"))?;

        let user_version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .with_context(|| format!("[golden] read user_version of {relative}"))?;
        lines.insert(format!("{relative}\tpragma\tuser_version\t{user_version}"));

        let mut stmt = conn
            .prepare(
                "SELECT type, name, COALESCE(sql, '') FROM sqlite_master
                 WHERE type IN ('table','index','trigger')",
            )
            .with_context(|| format!("[golden] prepare sqlite_master scan of {relative}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .with_context(|| format!("[golden] scan sqlite_master of {relative}"))?;
        for row in rows {
            let (kind, name, sql) = row.context("[golden] read sqlite_master row")?;
            lines.insert(format!(
                "{relative}\t{kind}\t{name}\t{}",
                normalize_sql(&sql)
            ));
        }
    }

    tracing::debug!(objects = lines.len(), "[golden] manifest built");
    Ok(lines)
}

/// Render a manifest as the committed file format: one line per object,
/// newline-separated, trailing newline.
pub fn render_manifest(manifest: &BTreeSet<String>) -> String {
    let mut out = manifest.iter().cloned().collect::<Vec<_>>().join("\n");
    out.push('\n');
    out
}

/// Parse a committed manifest file back into a set, ignoring blank lines and
/// `#` comments.
pub fn parse_manifest(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_sql_ignores_formatting_but_not_structure() {
        assert_eq!(
            normalize_sql("CREATE TABLE t (\n  a TEXT,\n  b INTEGER\n)"),
            normalize_sql("CREATE TABLE t ( a TEXT, b INTEGER )")
        );
        assert_ne!(
            normalize_sql("CREATE TABLE t (a TEXT)"),
            normalize_sql("CREATE TABLE t (a INTEGER)")
        );
    }

    #[test]
    fn manifest_round_trips_through_render_and_parse() {
        let manifest: BTreeSet<String> = [
            "a\ttable\tx\tCREATE TABLE x (i INT)",
            "a\tpragma\tuser_version\t0",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        assert_eq!(parse_manifest(&render_manifest(&manifest)), manifest);
    }

    #[test]
    fn parse_manifest_skips_comments_and_blanks() {
        let parsed = parse_manifest("# header\n\na\ttable\tx\tCREATE TABLE x (i INT)\n");
        assert_eq!(parsed.len(), 1);
    }
}
