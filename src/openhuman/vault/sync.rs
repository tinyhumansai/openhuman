//! Walk a vault's root directory and ingest changed/new files into the
//! memory_tree backend.
//!
//! # Memory-tree pipeline (not the legacy `memory_docs` path)
//!
//! Each file is ingested via [`crate::openhuman::memory::ingest_pipeline::ingest_document`]
//! against a stable `source_id` of the form `vault:{vault_id}:{rel_path}`.
//! That populates `mem_tree_chunks` + `mem_tree_ingested_sources` — which is
//! what every modern retrieval surface (`memory.search`, `tree.read_chunk`,
//! `tree.browse`, the agent's recall path, summary trees) reads from.
//!
//! Prior to #2705 this path called `doc_ingest`, which routed through the
//! legacy `UnifiedMemory` backend and wrote to `memory_docs` instead. The
//! UI's "synced" message was technically correct (UnifiedMemory accepted the
//! writes) but invisible to retrieval. Migrating to the memory-tree pipeline
//! closes that silent-failure mode.
//!
//! ## Source-id semantics
//!
//! - `source_id = vault:{vault_id}:{rel_path}` is stable for a given file path
//!   in a given vault. The pipeline's `already_ingested(SourceKind::Document,
//!   source_id)` gate is content-blind, so for content updates the vault
//!   layer must delete the prior chunks (via [`delete_chunks_by_source`])
//!   *before* re-ingesting, otherwise the new content is short-circuited.
//! - The vault's own per-file `content_hash` ledger entry is the authoritative
//!   "did the bytes change?" check — when it matches we skip the pipeline
//!   entirely (no delete, no re-ingest).
//! - When a previously-ledger'd file disappears from the walk, the same
//!   `delete_chunks_by_source` cleans up the orphan rows so memory_tree
//!   stays in sync with the on-disk vault.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{TimeZone, Utc};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::openhuman::config::Config;
use crate::openhuman::memory::ingest_pipeline::{self, IngestResult};
use crate::openhuman::memory::ops::{doc_delete, DeleteDocParams};
use crate::openhuman::memory_store::chunks::store::delete_chunks_by_source;
use crate::openhuman::memory_store::chunks::types::SourceKind;
use crate::openhuman::memory_sync::canonicalize::document::DocumentInput;

use super::state;
use super::store;
use super::types::{Vault, VaultFile, VaultFileStatus, VaultSyncReport};

/// Build the memory-tree source_id for one file in a vault. Stable across
/// re-syncs of the same `(vault, rel_path)`, so the pipeline's idempotency
/// gate works correctly and the vault ledger can map back to chunks for
/// cleanup.
fn vault_source_id(vault_id: &str, rel_path: &str) -> String {
    format!("vault:{vault_id}:{rel_path}")
}

/// Built-in exclude patterns we never traverse. Kept tiny and obvious.
const BUILTIN_EXCLUDE_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".cache",
    ".venv",
    "__pycache__",
    ".DS_Store",
];

/// Max single-file size we read into memory for ingestion (5 MiB).
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

/// Number of files to ingest concurrently.
///
/// Bounded to avoid overwhelming the embedding API while still parallelising
/// the dominant network cost.  Matches the codebase's existing `buffer_unordered`
/// patterns (see `extract_tool.rs` and `cron/scheduler.rs`).
const SYNC_CONCURRENCY: usize = 4;

/// File extensions we currently extract as plain UTF-8.
pub fn supported_extension(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "md" | "mdx"
            | "txt"
            | "rst"
            | "json"
            | "yaml"
            | "yml"
            | "toml"
            | "csv"
            | "html"
            | "htm"
            | "rs"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "py"
            | "go"
            | "java"
            | "rb"
            | "php"
            | "sh"
            | "bash"
            | "zsh"
            | "sql"
            | "css"
            | "scss"
            | "swift"
            | "kt"
            | "c"
            | "cc"
            | "cpp"
            | "h"
            | "hpp"
            | "log"
    )
}

/// A file that survived discovery and needs content read + ingestion.
struct FileToProcess {
    rel_path: String,
    title: String,
    path: PathBuf,
    mtime_ms: i64,
    bytes: u64,
    ext: String,
    /// Content hash from the previous successful sync, for secondary dedup.
    prev_hash: Option<String>,
    /// Vault id for tags and state updates.
    vault_id: String,
}

/// Outcome of attempting to ingest one file.
enum IngestFileResult {
    Ingested {
        rel_path: String,
        document_id: String,
        hash: String,
        mtime_ms: i64,
        bytes: u64,
    },
    /// Content was read but the hash matched the previous ingest — skip ledger write.
    Unchanged {
        rel_path: String,
    },
    Failed {
        rel_path: String,
        error: String,
    },
}

/// Read `file.path`, hash it, and route it through the memory-tree ingestion
/// pipeline. Skips the pipeline entirely when content is unchanged from the
/// previous successful sync.
///
/// Runs inside `buffer_unordered` so multiple files are in flight at once.
async fn process_file(config: Arc<Config>, file: FileToProcess) -> IngestFileResult {
    let content = match tokio::fs::read_to_string(&file.path).await {
        Ok(c) => c,
        Err(err) => {
            return IngestFileResult::Failed {
                rel_path: file.rel_path,
                error: format!("read failed: {err}"),
            };
        }
    };
    let hash = sha256_hex(&content);

    // Secondary dedup: content didn't change even if mtime did (e.g. `touch`).
    if file.prev_hash.as_deref() == Some(hash.as_str()) {
        log::trace!(
            "[vault] sync: hash-match skip path={} source_id={}",
            file.rel_path,
            vault_source_id(&file.vault_id, &file.rel_path),
        );
        return IngestFileResult::Unchanged {
            rel_path: file.rel_path,
        };
    }

    let source_id = vault_source_id(&file.vault_id, &file.rel_path);

    // For content updates (prev_hash was Some but didn't match), the
    // memory-tree pipeline's `already_ingested` gate would short-circuit
    // the new content because the source_id is stable per file path. Drop
    // the old chunks first so the re-ingest actually runs.
    if file.prev_hash.is_some() {
        let cfg_for_blocking = Arc::clone(&config);
        let source_for_blocking = source_id.clone();
        let delete_result = tokio::task::spawn_blocking(move || {
            delete_chunks_by_source(
                &cfg_for_blocking,
                SourceKind::Document,
                &source_for_blocking,
            )
        })
        .await;
        match delete_result {
            Ok(Ok(removed)) => {
                log::debug!(
                    "[vault] sync: re-ingest cleanup path={} source_id={} removed_chunks={}",
                    file.rel_path,
                    source_id,
                    removed
                );
            }
            Ok(Err(err)) => {
                // Failing the delete pre-empts a corrupt-state re-ingest —
                // surface as Failed instead of silently leaving stale rows
                // alongside new ones.
                return IngestFileResult::Failed {
                    rel_path: file.rel_path,
                    error: format!("delete_chunks_by_source failed before re-ingest: {err}"),
                };
            }
            Err(join_err) => {
                return IngestFileResult::Failed {
                    rel_path: file.rel_path,
                    error: format!("delete task join error: {join_err}"),
                };
            }
        }
    }

    // Pipeline modified_at = file mtime, so chunk metadata reflects when
    // the user last touched the file rather than when sync ran.
    let modified_at = Utc
        .timestamp_millis_opt(file.mtime_ms)
        .single()
        .unwrap_or_else(Utc::now);

    let doc = DocumentInput {
        provider: "vault".to_string(),
        title: file.title,
        body: content,
        modified_at,
        source_ref: Some(file.rel_path.clone()),
    };
    let tags = vec![
        format!("vault:{}", file.vault_id),
        format!("ext:{}", file.ext),
    ];

    // `&config` deref-coerces the `Arc<Config>` to `&Config`; the pipeline
    // owns no Config references beyond this call, so the ref-count survives
    // the await without an explicit clone.
    match ingest_pipeline::ingest_document(&config, &source_id, "vault", tags, doc).await {
        Ok(IngestResult {
            chunks_written,
            already_ingested,
            ..
        }) => {
            // The delete-first guard above prevents `already_ingested` on the
            // normal content-update path. If we still see it here it means
            // the vault ledger and `mem_tree_ingested_sources` are out of
            // sync (ledger wiped while the memory_tree row survived, or vice
            // versa) — the ledger gets a fresh row, but nothing new reaches
            // retrieval. That's the exact false-success mode this PR set out
            // to kill, so surface it loudly instead of swallowing it.
            if already_ingested && chunks_written == 0 {
                log::warn!(
                    "[vault] sync: ledger↔memory_tree desync detected — \
                     `already_ingested=true` for source_id={source_id} \
                     (path={}) but ledger had no matching row; no new chunks \
                     reached retrieval. Manual `delete_chunks_by_source` + \
                     resync may be required.",
                    file.rel_path,
                );
            } else {
                log::debug!(
                    "[vault] sync: ingested path={} source_id={} chunks_written={} already_ingested={}",
                    file.rel_path,
                    source_id,
                    chunks_written,
                    already_ingested,
                );
            }
            IngestFileResult::Ingested {
                rel_path: file.rel_path,
                document_id: source_id,
                hash,
                mtime_ms: file.mtime_ms,
                bytes: file.bytes,
            }
        }
        Err(err) => IngestFileResult::Failed {
            rel_path: file.rel_path,
            error: format!("ingest_document failed: {err}"),
        },
    }
}

/// Walk `vault.root_path`, ingest new/changed files into memory, delete docs
/// whose source files vanished, and record per-file state in the ledger.
pub async fn sync_vault(config: &Config, vault: &Vault) -> VaultSyncReport {
    let started = Utc::now();
    let mut report = VaultSyncReport {
        vault_id: vault.id.clone(),
        ..Default::default()
    };

    let root = PathBuf::from(&vault.root_path);
    if !root.is_dir() {
        report
            .errors
            .push(format!("root_path is not a directory: {}", vault.root_path));
        report.duration_ms = (Utc::now() - started).num_milliseconds();
        return report;
    }

    // Snapshot existing ledger so we can compute deletions at the end.
    let existing = match store::list_files(config, &vault.id) {
        Ok(rows) => rows,
        Err(err) => {
            report.errors.push(format!("ledger read failed: {err}"));
            return report;
        }
    };
    let mut seen: HashSet<String> = HashSet::new();
    let by_path: std::collections::HashMap<String, VaultFile> = existing
        .iter()
        .map(|f| (f.rel_path.clone(), f.clone()))
        .collect();

    let user_includes: Vec<String> = vault
        .include_globs
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let user_excludes: Vec<String> = vault
        .exclude_globs
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();

    log::debug!(
        "[vault] sync: entry id={} root={:?} ledger_rows={} includes={} excludes={}",
        vault.id,
        vault.root_path,
        existing.len(),
        user_includes.len(),
        user_excludes.len(),
    );

    // Prune builtin-excluded directory subtrees at traversal time so we never
    // descend into node_modules / target / .git etc.
    let walker = WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if !e.file_type().is_dir() {
                return true;
            }
            e.file_name()
                .to_str()
                .map(|name| !BUILTIN_EXCLUDE_DIRS.contains(&name))
                .unwrap_or(true)
        });

    // ── Phase 1: Discovery (sequential, no content reads) ───────────────────
    let mut candidates: Vec<FileToProcess> = Vec::new();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                log::debug!("[vault] sync: walk error err={err}");
                report.errors.push(format!("walk error: {err}"));
                continue;
            }
        };

        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let rel_path = match path.strip_prefix(&root) {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(_) => continue,
        };
        let rel_path_lc = rel_path.to_ascii_lowercase();

        // Defence-in-depth: filter_entry above prunes subtrees, but a future
        // refactor that drops it shouldn't silently let excluded files through.
        if path_is_inside_excluded_dir(path, &root) {
            continue;
        }
        if !user_includes.is_empty() && !user_includes.iter().any(|pat| rel_path_lc.contains(pat)) {
            continue;
        }
        if user_excludes.iter().any(|pat| rel_path_lc.contains(pat)) {
            continue;
        }

        report.scanned += 1;

        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if !supported_extension(&ext) {
            report.skipped_unsupported += 1;
            seen.insert(rel_path.clone());
            continue;
        }

        let metadata = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(err) => {
                report.failed += 1;
                report
                    .errors
                    .push(format!("{rel_path}: stat failed: {err}"));
                continue;
            }
        };
        if metadata.len() > MAX_FILE_BYTES {
            report.skipped_unsupported += 1;
            report.errors.push(format!(
                "{rel_path}: skipped — {} bytes exceeds {MAX_FILE_BYTES}",
                metadata.len()
            ));
            seen.insert(rel_path.clone());
            continue;
        }

        let mtime_ms = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        seen.insert(rel_path.clone());

        // Fast-path mtime dedup: if both mtime and previous hash matched we can
        // skip content reads entirely.  The concurrent phase does a secondary
        // hash-based check for files whose mtime changed but content didn't.
        if let Some(prev) = by_path.get(&rel_path) {
            if prev.status == VaultFileStatus::Ok && prev.mtime_ms == mtime_ms {
                report.unchanged += 1;
                continue;
            }
        }

        let title = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&rel_path)
            .to_string();
        let prev = by_path.get(&rel_path);

        candidates.push(FileToProcess {
            rel_path,
            title,
            path: path.to_path_buf(),
            mtime_ms,
            bytes: metadata.len(),
            ext,
            prev_hash: prev.map(|p| p.content_hash.clone()),
            vault_id: vault.id.clone(),
        });
    }

    log::debug!(
        "[vault] sync: discovery done id={} scanned={} unchanged={} to_ingest={}",
        vault.id,
        report.scanned,
        report.unchanged,
        candidates.len(),
    );

    // Update shared state with total count so the frontend can show progress.
    state::update_progress(&vault.id, |s| {
        s.scanned = report.scanned;
        s.unchanged = report.unchanged;
        s.total = candidates.len() as u64;
    });

    // ── Phase 2: Concurrent ingestion ────────────────────────────────────────
    //
    // Each task takes an `Arc<Config>` so we share one allocation across all
    // candidates instead of deep-cloning the config per file. A 5k-file vault
    // therefore pays one `Config::clone()` + N atomic ref-count bumps, vs the
    // previous N full clones — measurably cheaper on cold backfills.
    let config_for_workers = Arc::new(config.clone());
    let results: Vec<IngestFileResult> = futures::stream::iter(candidates)
        .map(move |file| process_file(Arc::clone(&config_for_workers), file))
        .buffer_unordered(SYNC_CONCURRENCY)
        .collect()
        .await;

    // ── Phase 3: Process results (sequential ledger writes) ──────────────────
    for result in results {
        match result {
            IngestFileResult::Ingested {
                rel_path,
                document_id,
                hash,
                mtime_ms,
                bytes,
            } => {
                let file = VaultFile {
                    vault_id: vault.id.clone(),
                    rel_path: rel_path.clone(),
                    document_id,
                    content_hash: hash,
                    mtime_ms,
                    bytes,
                    ingested_at: Utc::now(),
                    status: VaultFileStatus::Ok,
                };
                if let Err(err) = store::upsert_file(config, &file) {
                    log::debug!("[vault] sync: ledger write failed path={rel_path} err={err}");
                    report
                        .errors
                        .push(format!("{rel_path}: ledger write failed: {err}"));
                }
                log::trace!("[vault] sync: ingested path={rel_path}");
                report.ingested += 1;
                state::update_progress(&vault.id, |s| s.ingested += 1);
            }
            IngestFileResult::Unchanged { rel_path } => {
                // Hash matched even though mtime changed — still a no-op.
                log::trace!("[vault] sync: hash-unchanged path={rel_path}");
                report.unchanged += 1;
            }
            IngestFileResult::Failed { rel_path, error } => {
                log::debug!("[vault] sync: ingest failed path={rel_path} err={error}");
                report.failed += 1;
                report
                    .errors
                    .push(format!("{rel_path}: ingest failed: {error}"));
                state::update_progress(&vault.id, |s| s.failed += 1);
            }
        }
    }

    // ── Phase 4: Deletions ────────────────────────────────────────────────────
    //
    // Files that vanished from the vault since the last sync. We drop their
    // memory-tree rows so retrieval never resurfaces deleted content. The
    // `delete_chunks_by_source` helper handles `mem_tree_chunks`,
    // `mem_tree_ingested_sources`, and the on-disk content sidecars in one
    // transaction.
    //
    // Hoist the `Arc<Config>` for the blocking deletes once instead of
    // re-cloning the full `Config` per vanished file (mirrors the Phase 2
    // worker-pool optimisation).
    let config_for_deletes = Arc::new(config.clone());
    for (path, prev) in by_path.iter() {
        if seen.contains(path) {
            continue;
        }

        // Two ledger generations to handle here:
        //
        // * Post-#2705 rows: `document_id` is already `vault:{id}:{rel_path}`,
        //   so the memory_tree `delete_chunks_by_source` call below cleans it
        //   up exactly.
        // * Pre-#2705 rows: `document_id` is a legacy UnifiedMemory id
        //   (`{ts}_{hex}`-shaped) whose chunks live in `memory_docs`, *not*
        //   in `mem_tree_*`. Recomputing the memory_tree source_id and
        //   running `delete_chunks_by_source` deletes nothing on those rows;
        //   without a parallel `doc_delete` the legacy data leaks until
        //   UnifiedMemory removal lands (#2585 follow-up). We do both during
        //   the migration window so vanished files actually go away.
        let stored_id = prev.document_id.clone();
        let is_legacy_ledger_row = !stored_id.starts_with("vault:");
        let source_id = if is_legacy_ledger_row {
            log::debug!(
                "[vault] sync: legacy ledger doc_id detected during delete \
                 path={path} stored_id={stored_id} — falling back to recomputed \
                 source_id + parallel UnifiedMemory doc_delete"
            );
            vault_source_id(&vault.id, path)
        } else {
            stored_id.clone()
        };

        // Legacy UnifiedMemory cleanup. Best-effort: a 404 / missing-doc
        // error on the legacy path shouldn't block the memory_tree delete
        // below, which is the canonical store going forward.
        if is_legacy_ledger_row {
            if let Err(err) = doc_delete(DeleteDocParams {
                namespace: vault.namespace.clone(),
                document_id: stored_id.clone(),
            })
            .await
            {
                log::debug!(
                    "[vault] sync: legacy doc_delete failed (likely already absent) \
                     path={path} document_id={stored_id} err={err} — continuing with memory_tree cleanup"
                );
            }
        }

        let cfg_for_blocking = Arc::clone(&config_for_deletes);
        let source_for_blocking = source_id.clone();
        let path_label = path.clone();
        let delete_result = tokio::task::spawn_blocking(move || {
            delete_chunks_by_source(
                &cfg_for_blocking,
                SourceKind::Document,
                &source_for_blocking,
            )
        })
        .await;
        match delete_result {
            Ok(Ok(removed)) => {
                log::debug!(
                    "[vault] sync: deleted vanished file path={} source_id={} removed_chunks={}",
                    path_label,
                    source_id,
                    removed
                );
            }
            Ok(Err(err)) => {
                log::debug!(
                    "[vault] sync: delete_chunks_by_source failed path={path_label} err={err}"
                );
                report
                    .errors
                    .push(format!("{path_label}: chunk delete failed: {err}"));
                continue;
            }
            Err(join_err) => {
                report
                    .errors
                    .push(format!("{path_label}: delete task join error: {join_err}"));
                continue;
            }
        }
        if let Err(err) = store::delete_file(config, &vault.id, path) {
            log::debug!("[vault] sync: ledger delete failed path={path} err={err}");
            report
                .errors
                .push(format!("{path}: ledger delete failed: {err}"));
            continue;
        }
        report.removed += 1;
        state::update_progress(&vault.id, |s| s.removed += 1);
    }

    if let Err(err) = store::touch_last_synced(config, &vault.id, Utc::now()) {
        log::debug!("[vault] sync: touch_last_synced failed err={err}");
    }
    report.duration_ms = (Utc::now() - started).num_milliseconds();
    log::debug!(
        "[vault] sync: exit id={} scanned={} ingested={} unchanged={} removed={} failed={} skipped={} duration_ms={}",
        vault.id,
        report.scanned,
        report.ingested,
        report.unchanged,
        report.removed,
        report.failed,
        report.skipped_unsupported,
        report.duration_ms,
    );
    report
}

fn path_is_inside_excluded_dir(path: &Path, root: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    for component in rel.components() {
        if let std::path::Component::Normal(os) = component {
            if let Some(name) = os.to_str() {
                if BUILTIN_EXCLUDE_DIRS.contains(&name) {
                    return true;
                }
            }
        }
    }
    false
}

fn sha256_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest.iter() {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod sync_tests {
    use super::*;
    use crate::openhuman::memory_store::chunks::store::{count_chunks, is_source_ingested};
    use crate::openhuman::vault::ops;
    use crate::openhuman::vault::VaultWriteState;
    use tempfile::TempDir;

    /// Test-config pattern mirrors `memory::sync_pipeline_e2e_test::test_config`:
    /// tempdir-scoped workspace + embeddings disabled so the pipeline doesn't
    /// require a live provider. Embedding-strict OFF lets the pipeline accept
    /// chunks even without a working embedder.
    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().expect("tempdir");
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

    fn sample_vault(id: &str, root: &Path) -> Vault {
        Vault {
            id: id.to_string(),
            name: "Test Vault".to_string(),
            root_path: root.to_string_lossy().to_string(),
            host_os: None,
            namespace: format!("vault:{id}"),
            include_globs: vec![],
            exclude_globs: vec![],
            created_at: Utc::now(),
            last_synced_at: None,
            file_count: 0,
            write_state: VaultWriteState::Writable,
            write_state_reason: None,
        }
    }

    /// **The #2705 regression test.**
    ///
    /// Before this fix, vault sync routed through `doc_ingest` →
    /// `UnifiedMemory::ingest_document` → `memory_docs` table. The
    /// `mem_tree_chunks` / `mem_tree_ingested_sources` tables — which every
    /// modern retrieval surface reads from — were left empty, and the UI's
    /// "synced" message gave users a false-success signal.
    ///
    /// This test pins the invariant that vault sync must populate the
    /// memory-tree backend so the silent-failure mode can't reappear. It
    /// asserts both the chunk row count goes up and that each per-file
    /// source_id is registered in `mem_tree_ingested_sources`.
    #[tokio::test]
    async fn sync_writes_to_memory_tree() {
        let (_tmp, cfg) = test_config();

        let vault_root = TempDir::new().expect("vault root");
        let vault = sample_vault("vault-2705", vault_root.path());

        // Insert the vault row first — the vault_files ledger has a FK to
        // vaults, so per-file writes during Phase 3 would silently rollback
        // otherwise.
        store::insert_vault(&cfg, &vault).expect("insert vault");

        // Two non-trivial markdown files so the chunker definitely produces
        // ≥1 chunk per file (minimal content can otherwise be canonicalised
        // into nothing).
        std::fs::write(
            vault_root.path().join("note-one.md"),
            "# Note One\n\nThis is a substantive note about Project Phoenix. \
             It mentions Alice as the owner and contains enough text to ensure \
             the chunker produces at least one chunk. Phoenix ships in Q3.",
        )
        .expect("write note-one.md");
        std::fs::write(
            vault_root.path().join("note-two.md"),
            "# Note Two\n\nDifferent content about Project Atlas. Bob owns this \
             one. The team plans a launch in Q4 after staging review. Atlas is \
             unrelated to Phoenix.",
        )
        .expect("write note-two.md");

        let chunks_before = count_chunks(&cfg).expect("count_chunks before");
        let report = sync_vault(&cfg, &vault).await;

        assert_eq!(
            report.failed, 0,
            "no files should fail in a clean test setup; errors: {:?}",
            report.errors
        );
        assert_eq!(
            report.ingested, 2,
            "both .md files should ingest; report: {report:?}"
        );

        // Core regression assertion: chunks landed in memory_tree (NOT
        // memory_docs). Pre-fix, chunks_after would equal chunks_before
        // because vault sync wrote to the legacy backend instead.
        let chunks_after = count_chunks(&cfg).expect("count_chunks after");
        assert!(
            chunks_after > chunks_before,
            "vault sync must populate mem_tree_chunks (#2705): {chunks_before} → {chunks_after}"
        );

        // Per-file source registration in mem_tree_ingested_sources.
        // `is_source_ingested` returns false on the legacy backend even after
        // a "successful" doc_ingest run.
        let cfg_for_blocking = cfg.clone();
        let registered = tokio::task::spawn_blocking(move || {
            (
                is_source_ingested(
                    &cfg_for_blocking,
                    SourceKind::Document,
                    "vault:vault-2705:note-one.md",
                )
                .unwrap_or(false),
                is_source_ingested(
                    &cfg_for_blocking,
                    SourceKind::Document,
                    "vault:vault-2705:note-two.md",
                )
                .unwrap_or(false),
            )
        })
        .await
        .expect("source-check task join");
        assert!(
            registered.0 && registered.1,
            "both source_ids must be registered in mem_tree_ingested_sources (#2705); \
             note-one={} note-two={}",
            registered.0,
            registered.1
        );

        // Vault ledger stores the memory-tree source_id, not a legacy
        // UnifiedMemory document_id. This is the contract the deletion
        // path relies on (`stored_id.starts_with("vault:")`).
        let ledger = store::list_files(&cfg, "vault-2705").expect("list_files");
        assert_eq!(ledger.len(), 2, "ledger should have one row per file");
        for entry in &ledger {
            assert!(
                entry.document_id.starts_with("vault:vault-2705:"),
                "ledger document_id must hold the memory-tree source_id (got {})",
                entry.document_id
            );
        }
    }

    /// Stable across re-syncs: a second sync with no content changes leaves
    /// the chunk count untouched (idempotency invariant) and reports every
    /// file as `unchanged`.
    #[tokio::test]
    async fn second_sync_with_no_changes_is_idempotent() {
        let (_tmp, cfg) = test_config();
        let vault_root = TempDir::new().expect("vault root");
        let vault = sample_vault("vault-idem", vault_root.path());

        store::insert_vault(&cfg, &vault).expect("insert vault");

        std::fs::write(
            vault_root.path().join("stable.md"),
            "# Stable\n\nThis note doesn't change between syncs. Phoenix Q3.",
        )
        .expect("write stable.md");

        let first = sync_vault(&cfg, &vault).await;
        assert_eq!(first.ingested, 1, "first sync ingests the file");
        let chunks_after_first = count_chunks(&cfg).expect("count after first");

        let second = sync_vault(&cfg, &vault).await;
        assert_eq!(
            second.unchanged, 1,
            "second sync should hash-skip the unchanged file"
        );
        assert_eq!(second.ingested, 0, "no re-ingest on unchanged content");
        let chunks_after_second = count_chunks(&cfg).expect("count after second");
        assert_eq!(
            chunks_after_first, chunks_after_second,
            "idempotent re-sync must not duplicate chunks"
        );
    }

    /// **Regression: `vault_remove(purge_memory=true)` must clear memory_tree.**
    ///
    /// Post-#2705, vault content lives in `mem_tree_chunks` /
    /// `mem_tree_ingested_sources` keyed by `vault:{id}:{rel_path}`. The
    /// pre-fix purge path only called `clear_namespace`, which targets the
    /// legacy `memory_docs` table that vault sync no longer writes to —
    /// removing a vault with purge would silently orphan every memory-tree
    /// row and retrieval would keep surfacing content from a deleted vault.
    /// This test pins the prefix-delete contract so the silent-failure mode
    /// can't reappear on the removal side.
    #[tokio::test]
    async fn vault_remove_with_purge_clears_memory_tree() {
        let (_tmp, cfg) = test_config();

        let vault_root = TempDir::new().expect("vault root");
        let vault = sample_vault("vault-remove-2720", vault_root.path());

        // Use the real `vault_create` op so the row goes through the same
        // path production callers exercise (and so namespace/host_os are
        // realistic). `vault_create` canonicalises root_path, so the
        // returned vault id is the one to operate on below.
        let created =
            ops::vault_create(&cfg, &vault.name, vault.root_path.as_str(), vec![], vec![])
                .await
                .expect("vault_create")
                .value;
        let vault_id = created.id.clone();

        std::fs::write(
            vault_root.path().join("doomed.md"),
            "# Doomed\n\nThis note exists only long enough to be purged \
             with its parent vault. Phoenix Q4.",
        )
        .expect("write doomed.md");

        let report = sync_vault(&cfg, &created).await;
        assert_eq!(
            report.failed, 0,
            "clean ingest; errors: {:?}",
            report.errors
        );
        assert_eq!(report.ingested, 1);

        let source_id = vault_source_id(&vault_id, "doomed.md");
        let chunks_before = count_chunks(&cfg).expect("count_chunks before remove");
        assert!(
            chunks_before > 0,
            "sanity: memory_tree should contain the vault chunks before remove"
        );
        let registered_before = {
            let cfg_clone = cfg.clone();
            let src = source_id.clone();
            tokio::task::spawn_blocking(move || {
                is_source_ingested(&cfg_clone, SourceKind::Document, &src).unwrap_or(false)
            })
            .await
            .expect("source-check join")
        };
        assert!(
            registered_before,
            "sanity: source must be registered pre-remove"
        );

        let outcome = ops::vault_remove(&cfg, &vault_id, true)
            .await
            .expect("vault_remove");
        let payload = outcome.value;
        assert_eq!(payload["removed"], serde_json::json!(true));
        assert_eq!(payload["purged"], serde_json::json!(true));
        let purged_chunks = payload["memory_tree_chunks_deleted"]
            .as_u64()
            .expect("memory_tree_chunks_deleted field");
        assert!(
            purged_chunks > 0,
            "purge must report removed chunks; payload={payload}"
        );

        // Core regression assertion: no memory_tree rows survive the purge.
        let registered_after = {
            let cfg_clone = cfg.clone();
            let src = source_id.clone();
            tokio::task::spawn_blocking(move || {
                is_source_ingested(&cfg_clone, SourceKind::Document, &src).unwrap_or(true)
            })
            .await
            .expect("source-check join")
        };
        assert!(
            !registered_after,
            "vault_remove(purge=true) must clear mem_tree_ingested_sources for source_id={source_id}"
        );
        let chunks_after = count_chunks(&cfg).expect("count_chunks after remove");
        assert!(
            chunks_after < chunks_before,
            "memory_tree chunks must shrink after purge: {chunks_before} → {chunks_after}"
        );
    }

    /// `vault_remove(purge_memory=false)` must leave memory_tree rows in
    /// place. Guards the boolean from silently flipping to always-purge.
    #[tokio::test]
    async fn vault_remove_without_purge_leaves_memory_tree_intact() {
        let (_tmp, cfg) = test_config();
        let vault_root = TempDir::new().expect("vault root");
        let vault = sample_vault("vault-keep-2720", vault_root.path());

        let created =
            ops::vault_create(&cfg, &vault.name, vault.root_path.as_str(), vec![], vec![])
                .await
                .expect("vault_create")
                .value;
        let vault_id = created.id.clone();

        std::fs::write(
            vault_root.path().join("kept.md"),
            "# Kept\n\nThis content should survive a no-purge vault removal. \
             Atlas Q1 plan.",
        )
        .expect("write kept.md");

        let report = sync_vault(&cfg, &created).await;
        assert_eq!(
            report.failed, 0,
            "clean ingest; errors: {:?}",
            report.errors
        );
        let chunks_before = count_chunks(&cfg).expect("count_chunks before");

        let outcome = ops::vault_remove(&cfg, &vault_id, false)
            .await
            .expect("vault_remove");
        let payload = outcome.value;
        assert_eq!(payload["removed"], serde_json::json!(true));
        assert_eq!(payload["purged"], serde_json::json!(false));
        assert_eq!(
            payload["memory_tree_chunks_deleted"],
            serde_json::json!(0),
            "no-purge removal must not touch memory_tree"
        );

        let chunks_after = count_chunks(&cfg).expect("count_chunks after");
        assert_eq!(
            chunks_before, chunks_after,
            "no-purge removal must leave chunk count unchanged"
        );
    }

    /// `vault_source_id` is stable across calls — this is what makes the
    /// ledger ↔ memory_tree link work for cleanup on file deletion and for
    /// the content-update delete-then-reingest dance.
    #[test]
    fn vault_source_id_is_stable_and_namespaced() {
        let a = vault_source_id("v1", "notes/foo.md");
        let b = vault_source_id("v1", "notes/foo.md");
        assert_eq!(a, b, "must be deterministic for the same (vault, path)");
        assert_eq!(a, "vault:v1:notes/foo.md");

        // Different vaults / paths produce different ids — defends against
        // the pipeline's `already_ingested` gate cross-contaminating
        // distinct files.
        assert_ne!(
            vault_source_id("v1", "notes/foo.md"),
            vault_source_id("v2", "notes/foo.md")
        );
        assert_ne!(
            vault_source_id("v1", "notes/foo.md"),
            vault_source_id("v1", "notes/bar.md")
        );
    }
}
