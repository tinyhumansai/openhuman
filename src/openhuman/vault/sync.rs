//! Walk a vault's root directory and ingest changed/new files into memory.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::Utc;
use futures::StreamExt;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::openhuman::config::Config;
use crate::openhuman::memory::ops::{doc_delete, doc_ingest, DeleteDocParams, IngestDocParams};

use super::state;
use super::store;
use super::types::{Vault, VaultFile, VaultFileStatus, VaultSyncReport};

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
    /// Document ID to update on re-ingest (keeps embedding lineage stable).
    existing_doc_id: Option<String>,
    /// Memory namespace (`vault:<id>`).
    namespace: String,
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

/// Read `file.path`, hash it, and call `doc_ingest` if the content changed.
///
/// This runs inside `buffer_unordered` so multiple files are in flight at once.
async fn process_file(file: FileToProcess) -> IngestFileResult {
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
        return IngestFileResult::Unchanged {
            rel_path: file.rel_path,
        };
    }

    let ingest_params = IngestDocParams {
        namespace: file.namespace,
        key: file.rel_path.clone(),
        title: file.title,
        content,
        source_type: "vault".to_string(),
        priority: "medium".to_string(),
        tags: vec![
            format!("vault:{}", file.vault_id),
            format!("ext:{}", file.ext),
        ],
        metadata: serde_json::json!({
            "vault_id": file.vault_id,
            "rel_path": file.rel_path,
            "mtime_ms": file.mtime_ms,
            "bytes": file.bytes,
        }),
        category: "user".to_string(),
        session_id: None,
        document_id: file.existing_doc_id,
        config: None,
    };

    match doc_ingest(ingest_params).await {
        Ok(outcome) => IngestFileResult::Ingested {
            rel_path: file.rel_path,
            document_id: outcome.value.document_id,
            hash,
            mtime_ms: file.mtime_ms,
            bytes: file.bytes,
        },
        Err(err) => IngestFileResult::Failed {
            rel_path: file.rel_path,
            error: err,
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
            existing_doc_id: prev.map(|p| p.document_id.clone()),
            namespace: vault.namespace.clone(),
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
    let results: Vec<IngestFileResult> = futures::stream::iter(candidates)
        .map(process_file)
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
    for (path, prev) in by_path.iter() {
        if seen.contains(path) {
            continue;
        }
        if let Err(err) = doc_delete(DeleteDocParams {
            namespace: vault.namespace.clone(),
            document_id: prev.document_id.clone(),
        })
        .await
        {
            log::debug!("[vault] sync: doc delete failed path={path} err={err}");
            report
                .errors
                .push(format!("{path}: doc delete failed: {err}"));
            continue;
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
