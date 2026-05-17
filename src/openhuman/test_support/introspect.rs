//! Read-only introspection RPCs for E2E specs.
//!
//! These mirror the access patterns specs need to verify that "the UI did
//! something" actually flowed all the way through to disk + the in-process
//! Rust state. They are intentionally narrow and read-only — no writes,
//! no side effects beyond a single config/file read.
//!
//! Like `test_reset`, the bearer-token requirement on `/rpc` keeps these
//! out of release-build reach (the per-launch token file is only written
//! in debug builds).

use std::path::{Path, PathBuf};

use serde::Serialize;
use tokio::fs;

use crate::openhuman::channels::providers::web::in_flight_entries_for_test;
use crate::openhuman::config::Config;
use crate::openhuman::wallet::prepared_quotes_for_test;
use crate::rpc::RpcOutcome;

/// Maximum bytes returned by `read_workspace_file`. Specs that need bigger
/// reads should chunk through `list_workspace_files` + multiple reads.
const READ_FILE_MAX_BYTES: u64 = 1024 * 1024; // 1 MiB

/// Maximum recursion depth for `list_workspace_files`.
const LIST_MAX_DEPTH: u32 = 6;

/// Reject any relative path containing a `..` component or that resolves
/// outside the workspace root. Returns the joined absolute path on success.
fn resolve_workspace_relative(workspace: &Path, rel: &str) -> Result<PathBuf, String> {
    let trimmed = rel.trim_start_matches('/');
    let candidate = workspace.join(trimmed);
    let canonical_root = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let canonical_candidate = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.clone());
    if !canonical_candidate.starts_with(&canonical_root) {
        return Err(format!(
            "rel_path {rel:?} escapes workspace root {}",
            workspace.display()
        ));
    }
    Ok(candidate)
}

async fn current_workspace_dir() -> Result<PathBuf, String> {
    let config = Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e}"))?;
    Ok(config.workspace_dir.clone())
}

// ── workspace_root ────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct WorkspaceRoot {
    pub path: String,
    pub exists: bool,
}

pub async fn workspace_root() -> Result<RpcOutcome<WorkspaceRoot>, String> {
    let dir = current_workspace_dir().await?;
    let exists = fs::try_exists(&dir).await.unwrap_or(false);
    Ok(RpcOutcome::single_log(
        WorkspaceRoot {
            path: dir.display().to_string(),
            exists,
        },
        format!("workspace_root: {} (exists={exists})", dir.display()),
    ))
}

// ── list_workspace_files ──────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ListEntry {
    pub rel_path: String,
    pub size: u64,
    pub is_dir: bool,
}

#[derive(Debug, Serialize)]
pub struct ListResult {
    pub root: String,
    pub entries: Vec<ListEntry>,
    pub truncated: bool,
}

async fn walk_dir(
    root: &Path,
    start: &Path,
    max_depth: u32,
    out: &mut Vec<ListEntry>,
    limit: usize,
) -> Result<bool, String> {
    // Explicit BFS stack avoids needing the async_recursion crate.
    let mut stack: Vec<(PathBuf, u32)> = vec![(start.to_path_buf(), 0)];
    while let Some((cur, depth)) = stack.pop() {
        if depth > max_depth {
            continue;
        }
        let mut rd = match fs::read_dir(&cur).await {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| format!("read_dir: {e}"))?
        {
            if out.len() >= limit {
                return Ok(true);
            }
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| path.display().to_string());
            // `symlink_metadata` does NOT follow links — a symlink
            // inside the workspace that points outside (or anywhere
            // else) would otherwise be reported, and if it resolved to
            // a directory we'd recurse into it. That would break the
            // workspace-only guarantee the RPC promises.
            let meta = match fs::symlink_metadata(&path).await {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            let is_dir = meta.is_dir();
            out.push(ListEntry {
                rel_path: rel,
                size: if is_dir { 0 } else { meta.len() },
                is_dir,
            });
            if is_dir && depth + 1 <= max_depth {
                stack.push((path, depth + 1));
            }
        }
    }
    Ok(false)
}

pub async fn list_workspace_files(
    rel_root: Option<String>,
    max_depth: Option<u32>,
) -> Result<RpcOutcome<ListResult>, String> {
    let workspace = current_workspace_dir().await?;
    let root = match rel_root.as_deref().filter(|s| !s.is_empty()) {
        Some(r) => resolve_workspace_relative(&workspace, r)?,
        None => workspace.clone(),
    };
    let depth = max_depth.unwrap_or(2).min(LIST_MAX_DEPTH);
    let mut entries = Vec::new();
    let truncated = walk_dir(&root, &root, depth, &mut entries, 2_000).await?;

    Ok(RpcOutcome::single_log(
        ListResult {
            root: root.display().to_string(),
            entries: entries.iter().cloned().collect(),
            truncated,
        },
        format!(
            "listed {} entries (depth={depth}, truncated={truncated}) under {}",
            entries.len(),
            root.display()
        ),
    ))
}

// `ListEntry` is `Clone`-able for the log line summarisation above.
impl Clone for ListEntry {
    fn clone(&self) -> Self {
        Self {
            rel_path: self.rel_path.clone(),
            size: self.size,
            is_dir: self.is_dir,
        }
    }
}

// ── read_workspace_file ───────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ReadFileResult {
    pub rel_path: String,
    pub size_on_disk: u64,
    pub returned_bytes: u64,
    pub truncated: bool,
    pub content_utf8: String,
}

pub async fn read_workspace_file(
    rel_path: String,
    max_bytes: Option<u64>,
) -> Result<RpcOutcome<ReadFileResult>, String> {
    let workspace = current_workspace_dir().await?;
    let abs = resolve_workspace_relative(&workspace, &rel_path)?;
    let meta = fs::metadata(&abs)
        .await
        .map_err(|e| format!("stat {}: {e}", abs.display()))?;
    if meta.is_dir() {
        return Err(format!("read_workspace_file: {rel_path} is a directory"));
    }
    let cap = max_bytes
        .unwrap_or(READ_FILE_MAX_BYTES)
        .min(READ_FILE_MAX_BYTES);
    let raw = fs::read(&abs)
        .await
        .map_err(|e| format!("read {}: {e}", abs.display()))?;
    let size_on_disk = raw.len() as u64;
    let truncated = size_on_disk > cap;
    let returned = if truncated {
        raw[..cap as usize].to_vec()
    } else {
        raw
    };
    // `returned_bytes` is the raw byte count read from disk before
    // lossy UTF-8 conversion — `from_utf8_lossy` substitutes U+FFFD
    // for invalid sequences, which can change the byte length. Specs
    // assert against this value to verify byte-accurate truncation.
    let returned_bytes = returned.len() as u64;
    let content_utf8 = String::from_utf8_lossy(&returned).into_owned();
    Ok(RpcOutcome::single_log(
        ReadFileResult {
            rel_path: rel_path.clone(),
            size_on_disk,
            returned_bytes,
            truncated,
            content_utf8,
        },
        format!(
            "read_workspace_file {rel_path}: size_on_disk={size_on_disk}, truncated={truncated}"
        ),
    ))
}

// ── in_flight_chats ───────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct InFlightEntryView {
    pub key: String,
    pub request_id: String,
}

#[derive(Debug, Serialize)]
pub struct InFlightResult {
    pub entries: Vec<InFlightEntryView>,
}

pub async fn in_flight_chats() -> Result<RpcOutcome<InFlightResult>, String> {
    let entries: Vec<InFlightEntryView> = in_flight_entries_for_test()
        .await
        .into_iter()
        .map(|(key, request_id)| InFlightEntryView { key, request_id })
        .collect();
    let count = entries.len();
    Ok(RpcOutcome::single_log(
        InFlightResult { entries },
        format!("in_flight_chats: {count} entries"),
    ))
}

// ── wallet_prepared_quotes ────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedQuotesResult {
    pub count: usize,
    pub quotes: Vec<crate::openhuman::wallet::PreparedTransaction>,
}

pub async fn wallet_prepared_quotes() -> Result<RpcOutcome<PreparedQuotesResult>, String> {
    let quotes = prepared_quotes_for_test();
    let count = quotes.len();
    Ok(RpcOutcome::single_log(
        PreparedQuotesResult { count, quotes },
        format!("wallet_prepared_quotes: {count} quotes"),
    ))
}
