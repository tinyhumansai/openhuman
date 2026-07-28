//! Layer-2 golden-workspace schema-parity harness (migration spec §0.3, parity
//! checklist "Layer 2").
//!
//! The Layer-1 asserters (`src/openhuman/tinycortex/parity.rs`) pin pure on-disk
//! *format* contracts (chunk ids, vector encoding, vault paths, signatures).
//! This is the Layer-2 **differential** guard: it stands up a real workspace
//! through the host's production memory surface (`memory::ops`) and asserts that
//! the two schema tiers that share the workspace **compose** correctly —
//!
//!   1. the **crate-owned substrate** the `tinycortex` chunk DB creates
//!      (`init_db` → `chunks/schema.rs`), and
//!   2. the **host-retained `UnifiedMemory` namespace-document tier**
//!      (`memory_store/namespace_store/*`),
//!
//! coexisting without collision (parity checklist P3/P5/P11/P12 — the W3 gate).
//! A store/tree cutover that reshaped, renamed, or dropped a table would strand
//! an existing user workspace; this fails here first.
//!
//! Design notes:
//! - **Path-agnostic.** It recursively scans *every* `*.db` under the temp
//!   workspace and unions their tables, so it does not care whether the tiers
//!   live in one DB file or several, nor exactly where the host client roots
//!   them.
//! - The crate chunk-DB init is additionally forced via
//!   `tinycortex::memory::chunks::with_connection` so the substrate schema is
//!   deterministic regardless of which subsystems the op flow happened to touch.
//! - The crate KV tier (`kv_global` / `kv_namespace`, crate `store/kv.rs`)
//!   attaches to the host `UnifiedMemory` connection via
//!   `KvStore::from_shared_connection`. These two table *names* are pinned as
//!   [`CRATE_KV_TABLES`], but a name-presence check alone is **not** a cutover
//!   guard: the host `UnifiedMemory::new` (`namespace_store/init.rs`) also
//!   `CREATE TABLE IF NOT EXISTS`es both names unconditionally on every workspace
//!   open, so the names would survive even if the crate KV store cut over to
//!   differently-named tables and stranded a user's persisted preferences. The
//!   real guard is therefore **functional**: [`assert_crate_kv_interop`] drives
//!   the production KV surface (`memory::ops::kv_{set,get}`, crate-backed via
//!   `KvStore::from_shared_connection`) for the global and namespace scopes,
//!   asserts the value round-trips, and asserts via read-only SQL that the write
//!   physically landed in the pinned `kv_global` / `kv_namespace` tables. A
//!   cutover that renamed or dropped the crate KV tables strands the write
//!   outside the pinned names and fails here.
//! - The standalone `VectorStore` tables (`vectors` / `store_meta`, crate
//!   `store/vectors/store.rs`) are deliberately **not** pinned: nothing on the
//!   host's live path opens that store — `VectorStore::open` has no non-test
//!   caller in either the crate or the host, and the live embedding path instead
//!   uses `mem_tree_chunk_embeddings` (crate substrate) plus the host
//!   `vector_chunks` tier — so those tables never appear in a real workspace.
//!   Pinning them would assert schema the shipped product never creates. Add
//!   them here only if the host ever wires the standalone vector store onto a
//!   workspace.
//!
//! Run with: `cargo test --test memory_golden_parity_e2e`

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use tempfile::tempdir;

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::memory::ops::{
    doc_put, kv_get, kv_set, memory_recall_context, memory_recall_memories, KvGetDeleteParams,
    KvSetParams, PutDocParams,
};
use openhuman_core::openhuman::memory::rpc_models::{RecallContextRequest, RecallMemoriesRequest};
use openhuman_core::openhuman::tinycortex::memory_config_from;

// ── Env isolation (mirrors memory_roundtrip_e2e) ─────────────────────────────

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        // SAFETY: only used under env_lock(), which serialises env mutation.
        unsafe { std::env::set_var(key, path.as_os_str()) };
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            // SAFETY: see set_to_path; teardown runs under the same env_lock().
            Some(v) => unsafe { std::env::set_var(self.key, v) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

/// Serialises tests: `HOME` + `OPENHUMAN_WORKSPACE` are process-global.
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock poisoned")
}

// ── Expected schema tiers (authoritative names from the two engines) ─────────

/// The crate chunk-DB substrate created by `init_db` (`chunks/schema.rs`). These
/// are the tables the tinycortex store owns and must preserve byte-for-byte
/// across every W3+ cutover.
const CRATE_CHUNK_SCHEMA_TABLES: &[&str] = &[
    "mem_tree_chunks",
    "mem_tree_chunk_embeddings",
    "mem_tree_chunk_reembed_skipped",
    "mem_tree_score",
    "mem_tree_entity_index",
    "mem_tree_entity_edges",
    "mem_tree_trees",
    "mem_tree_summaries",
    "mem_tree_summary_embeddings",
    "mem_tree_summary_reembed_skipped",
    "mem_tree_buffers",
    "mem_tree_entity_hotness",
    "mem_tree_jobs",
    "mem_tree_ingested_sources",
    "mcp_writes",
];

/// The host-retained `UnifiedMemory` namespace-document tier
/// (`memory_store/namespace_store/*`) — stays host, coexists in the shared workspace.
const HOST_UNIFIED_TABLES: &[&str] = &[
    "memory_docs",
    "graph_global",
    "graph_namespace",
    "episodic_log",
    "event_log",
    "event_embeddings",
    "conversation_segments",
    "segment_embeddings",
    "vector_chunks",
    "user_profile",
];

/// The crate KV tier (`kv_global` + `kv_namespace`, crate `store/kv.rs`) that
/// rides the host `UnifiedMemory` connection via `KvStore::from_shared_connection`.
/// These names are the *targets* the production KV write path must land in;
/// [`assert_crate_kv_interop`] is what proves it does. The harness guards against
/// these tables being **renamed or dropped** by the crate KV store — not against
/// arbitrary in-place schema reshaping (a column/index change that preserves the
/// names and the `key` / `value_json` columns the API reads would still pass).
const CRATE_KV_TABLES: &[&str] = &["kv_global", "kv_namespace"];

// ── Schema scan helpers (path-agnostic, read-only) ───────────────────────────

fn collect_db_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_db_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("db") {
            out.push(path);
        }
    }
}

/// Union of every user table across every `*.db` under `ws` (read-only opens;
/// SQLite-internal `sqlite_%` tables excluded).
fn tables_in_workspace(ws: &Path) -> BTreeSet<String> {
    let mut dbs = Vec::new();
    collect_db_files(ws, &mut dbs);

    let mut tables = BTreeSet::new();
    for db in dbs {
        let Ok(conn) =
            rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        else {
            continue;
        };
        let Ok(mut stmt) = conn.prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        ) else {
            continue;
        };
        let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) else {
            continue;
        };
        for name in rows.flatten() {
            tables.insert(name);
        }
    }
    tables
}

/// Read-only: does any `*.db` under `ws` hold a row keyed `key` in `table`?
///
/// Used to prove that a production KV write physically landed in a *pinned* table
/// name rather than one the crate KV store may have cut over to. `table` and
/// `key` are harness constants (never external input), so the interpolated
/// `table` name carries no injection surface. A missing table or unreadable DB
/// counts as "not present" and the scan moves on.
fn kv_row_present(ws: &Path, table: &str, key: &str) -> bool {
    let mut dbs = Vec::new();
    collect_db_files(ws, &mut dbs);
    for db in dbs {
        let Ok(conn) =
            rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        else {
            continue;
        };
        let sql = format!("SELECT COUNT(*) FROM \"{table}\" WHERE key = ?1");
        if let Ok(count) = conn.query_row(&sql, [key], |row| row.get::<_, i64>(0)) {
            if count > 0 {
                return true;
            }
        }
    }
    false
}

/// The crate KV tier's real cutover guard (see the module-level design note and
/// [`CRATE_KV_TABLES`]). A name-presence check alone cannot detect a crate KV
/// cutover because the host `UnifiedMemory::new` recreates `kv_global` /
/// `kv_namespace` unconditionally; this instead drives the production KV surface
/// (crate-backed via `KvStore::from_shared_connection`) for the global and
/// namespace scopes, asserts the value round-trips, and asserts via read-only SQL
/// that each write physically landed in the pinned tables. A cutover that renamed
/// or dropped the crate KV tables strands the write outside the pinned names and
/// trips one of these asserts.
async fn assert_crate_kv_interop(workspace: &Path, namespace: &str, tables: &BTreeSet<String>) {
    eprintln!(
        "[golden-parity][kv] validating crate KV interop over pinned tables {:?}",
        CRATE_KV_TABLES
    );

    // Cheap coexistence pre-check: the pinned tables materialised at all.
    let missing_kv: Vec<&str> = CRATE_KV_TABLES
        .iter()
        .copied()
        .filter(|t| !tables.contains(*t))
        .collect();
    assert!(
        missing_kv.is_empty(),
        "crate KV-tier tables missing from the workspace: {missing_kv:?}; found: {tables:?}"
    );

    let key = "golden-parity-kv-canary";
    let value = serde_json::json!({ "pref": "golden-parity", "v": 1 });

    // ── Global scope: write via production kv_set, read back, prove it landed
    //    in the pinned `kv_global` table. ──
    eprintln!("[golden-parity][kv] global-scope write via production kv_set");
    kv_set(KvSetParams {
        namespace: None,
        key: key.to_string(),
        value: value.clone(),
    })
    .await
    .expect("crate KV global set");
    let got_global = kv_get(KvGetDeleteParams {
        namespace: None,
        key: key.to_string(),
    })
    .await
    .expect("crate KV global get");
    assert_eq!(
        got_global.value,
        Some(value.clone()),
        "crate KV global round-trip lost the value (adapter cannot read back its own write)"
    );
    assert!(
        kv_row_present(workspace, "kv_global", key),
        "crate KV global write did not land in the pinned `kv_global` table — the KV store cut over to a differently-named table and would strand existing preferences"
    );

    // ── Namespace scope: same, against the pinned `kv_namespace` table. ──
    eprintln!("[golden-parity][kv] namespace-scope write via production kv_set");
    kv_set(KvSetParams {
        namespace: Some(namespace.to_string()),
        key: key.to_string(),
        value: value.clone(),
    })
    .await
    .expect("crate KV namespace set");
    let got_ns = kv_get(KvGetDeleteParams {
        namespace: Some(namespace.to_string()),
        key: key.to_string(),
    })
    .await
    .expect("crate KV namespace get");
    assert_eq!(
        got_ns.value,
        Some(value.clone()),
        "crate KV namespace round-trip lost the value (adapter cannot read back its own write)"
    );
    assert!(
        kv_row_present(workspace, "kv_namespace", key),
        "crate KV namespace write did not land in the pinned `kv_namespace` table — the KV store cut over to a differently-named table and would strand existing preferences"
    );

    eprintln!(
        "[golden-parity][kv] crate KV interop verified — round-trip ok and writes landed in {:?}",
        CRATE_KV_TABLES
    );
}

fn put_params(ns: &str) -> PutDocParams {
    PutDocParams {
        namespace: ns.to_string(),
        key: "golden-parity-canary".to_string(),
        title: "Golden parity canary".to_string(),
        content: "TinyCortex golden-workspace schema-parity canary fact".to_string(),
        source_type: "doc".to_string(),
        priority: "medium".to_string(),
        tags: Vec::new(),
        metadata: serde_json::Value::Null,
        category: "core".to_string(),
        session_id: None,
        document_id: None,
    }
}

/// Drive the real production surface so both schema tiers initialise, then force
/// the crate substrate init to make the chunk-DB schema deterministic. Returns
/// the union of tables observed across the workspace.
async fn init_and_scan(ns: &str, workspace: &Path) -> BTreeSet<String> {
    // Host unified tier + retrieval (production path).
    doc_put(put_params(ns)).await.expect("doc_put");
    let _ = memory_recall_memories(RecallMemoriesRequest {
        namespace: ns.to_string(),
        min_retention: None,
        as_of: None,
        limit: Some(10),
        max_chunks: None,
        top_k: None,
    })
    .await
    .expect("recall_memories");
    let _ = memory_recall_context(RecallContextRequest {
        namespace: ns.to_string(),
        include_references: Some(true),
        limit: Some(10),
        max_chunks: None,
    })
    .await
    .expect("recall_context");

    // Force the crate chunk-DB substrate init (deterministic — creates the full
    // chunks/schema.rs table set regardless of what the ops above touched).
    let mc = memory_config_from(&Config::default(), workspace.to_path_buf());
    tinycortex::memory::chunks::with_connection(&mc, |_conn| Ok(())).expect("crate chunk-DB init");

    tables_in_workspace(workspace)
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// P3/P5/P11/P12 — the crate substrate and the host `UnifiedMemory` tier both
/// initialise into the shared workspace without collision. Any cutover that
/// renames/drops one of these tables fails here before it can strand a real
/// user workspace.
#[tokio::test]
async fn golden_workspace_composes_substrate_and_unified_tiers() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("mkdir workspace");
    let _ws = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace);

    let tables = init_and_scan("golden-parity-e2e", &workspace).await;

    // Full schema dump for review / manifest capture in the test log.
    eprintln!(
        "[golden-parity] workspace tables ({}): {:?}",
        tables.len(),
        tables
    );

    let missing_substrate: Vec<&str> = CRATE_CHUNK_SCHEMA_TABLES
        .iter()
        .copied()
        .filter(|t| !tables.contains(*t))
        .collect();
    assert!(
        missing_substrate.is_empty(),
        "crate chunk-DB substrate tables missing from the workspace: {missing_substrate:?}; found: {tables:?}"
    );

    let missing_unified: Vec<&str> = HOST_UNIFIED_TABLES
        .iter()
        .copied()
        .filter(|t| !tables.contains(*t))
        .collect();
    assert!(
        missing_unified.is_empty(),
        "host UnifiedMemory tables missing from the workspace: {missing_unified:?}; found: {tables:?}"
    );

    // Crate KV tier: a functional interop guard, not a name-presence check.
    // (Name presence alone is satisfied by the host `UnifiedMemory::new` init and
    // cannot detect a crate KV cutover — see [`assert_crate_kv_interop`].)
    assert_crate_kv_interop(&workspace, "golden-parity-e2e", &tables).await;

    // Coexistence: both tiers are present in the same workspace (P12).
    assert!(
        tables.contains("mem_tree_chunks") && tables.contains("memory_docs"),
        "both the crate substrate and the host unified tier must coexist"
    );

    // Comparator 5 (idempotent re-open): keep this in the same test because
    // the production memory client is process-global and deliberately binds
    // to its first workspace. Separate tests with separate temp workspaces can
    // therefore pass or fail depending on test scheduling.
    let reopened = init_and_scan("golden-parity-e2e", &workspace).await;

    assert_eq!(
        tables, reopened,
        "re-running the flow changed the workspace table set (schema churn on re-open)"
    );
}
