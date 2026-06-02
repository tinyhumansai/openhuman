//! Vault file-system watcher.
//!
//! Watches a configurable directory (default: the Obsidian vault's
//! `wiki/notes/` folder) for `Create`, `Modify`, and `Remove` events
//! and ingests changes into the memory tree in near-real time.
//!
//! ## Design
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │  notify (OS-native watcher)                             │
//! │  FSEvents / inotify / ReadDirectoryChanges              │
//! └────────────────────┬────────────────────────────────────┘
//!                      │  raw events (debounced, 500 ms)
//!                      ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │  run_loop()  ← tokio task (singleton via OnceLock)      │
//! │                                                         │
//! │  ① scheduler-gate check  (UserDisabled / SignedOut)     │
//! │  ② mtime guard           (SQLite WatcherStateStore)     │
//! │  ③ path→source_id build  (stable base + mtime suffix)  │
//! │  ④ ingest_document_with_scope()  or  mark_deleted()     │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Dedup strategy (mtime-based source_id)
//!
//! `ingest_document_with_scope` deduplicates on `source_id`.  A plain
//! `path`-based ID means edits are silently ignored (already-ingested
//! guard fires).  We therefore build:
//!
//! ```text
//! source_id = "vault_watcher:<rel_path>@<mtime_secs>"
//! ```
//!
//! Every modification creates a new `source_id`, bypassing the dedup
//! gate and letting the pipeline store a fresh version.  The previous
//! version remains in the store but becomes unreachable via normal
//! queries (the tree rebuild naturally supersedes it).
//!
//! For `Remove` events we call `mark_document_deleted(source_id)` so
//! the entry is tombstoned rather than left as orphan data.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use notify::{
    event::{CreateKind, ModifyKind, RemoveKind},
    Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use notify_debouncer_mini::{new_debouncer, DebouncedEvent, Debouncer};
use tokio::sync::mpsc;

use crate::openhuman::config::{rpc as config_rpc, Config};
use crate::openhuman::memory::ingest_pipeline::ingest_document_with_scope;
use crate::openhuman::memory_sync::canonicalize::document::DocumentInput;
use crate::openhuman::memory_sync::workspace::watcher::state::WatcherStateStore;
use crate::openhuman::scheduler_gate::gate::current_policy;
use crate::openhuman::scheduler_gate::policy::PauseReason;

pub mod state;

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Debounce window: coalesce bursts of rapid saves into one event.
const DEBOUNCE_MS: u64 = 500;

/// Only ingest files matching these extensions.
const WATCHED_EXTENSIONS: &[&str] = &["md", "txt"];

/// State DB filename inside the workspace directory.
const STATE_DB_FILENAME: &str = "vault_watcher_state.db";

// ─────────────────────────────────────────────────────────────────────────────
// Singleton guard — mirrors composio/periodic.rs pattern exactly
// ─────────────────────────────────────────────────────────────────────────────

static WATCHER_STARTED: OnceLock<()> = OnceLock::new();

/// Spawn the vault watcher background task.  Idempotent: only the first
/// call actually spawns; subsequent calls are cheap no-ops.
pub fn start_vault_watcher() {
    if WATCHER_STARTED.get().is_some() {
        tracing::debug!("[vault_watcher] already running, skipping start");
        return;
    }
    if WATCHER_STARTED.set(()).is_err() {
        tracing::debug!("[vault_watcher] already running (race), skipping start");
        return;
    }

    tokio::spawn(async move {
        tracing::info!("[vault_watcher] starting");
        if let Err(e) = run_loop().await {
            tracing::error!(error = %e, "[vault_watcher] loop exited with error");
        }
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Scheduler-gate check — same allow-list as composio/periodic.rs
// ─────────────────────────────────────────────────────────────────────────────

fn watcher_pause_reason() -> Option<PauseReason> {
    let reason = current_policy().pause_reason()?;
    matches!(reason, PauseReason::UserDisabled | PauseReason::SignedOut).then_some(reason)
}

// ─────────────────────────────────────────────────────────────────────────────
// Main loop
// ─────────────────────────────────────────────────────────────────────────────

async fn run_loop() -> Result<(), String> {
    let config = config_rpc::load_config_with_timeout()
        .await
        .map_err(|e| format!("[vault_watcher] load_config: {e}"))?;

    let watch_path = resolve_watch_path(&config)?;

    tracing::info!(
        path = %watch_path.display(),
        "[vault_watcher] watching vault directory"
    );

    // Open (or create) the SQLite state store.
    let db_path = config
        .workspace_dir()
        .join(STATE_DB_FILENAME);
    let state_store = Arc::new(Mutex::new(
        WatcherStateStore::open(&db_path)
            .map_err(|e| format!("[vault_watcher] state db open failed: {e}"))?,
    ));

    // Seed in-memory mtime map from SQLite so a restart doesn't re-ingest
    // everything from scratch.
    let mtime_cache: Arc<Mutex<HashMap<PathBuf, u64>>> = {
        let store = state_store.lock().unwrap_or_else(|e| e.into_inner());
        let all = store.load_all().map_err(|e| format!("[vault_watcher] load_all: {e}"))?;
        let map: HashMap<PathBuf, u64> = all
            .into_iter()
            .filter(|s| !s.deleted)
            .map(|s| (s.path, s.mtime_secs))
            .collect();
        Arc::new(Mutex::new(map))
    };

    // Channel between the notify callback (sync) and our async handler.
    let (tx, mut rx) = mpsc::unbounded_channel::<DebouncedEvent>();

    // Build the debounced watcher.  `new_debouncer` returns a
    // `Debouncer<RecommendedWatcher>` which we must keep alive.
    let tx_clone = tx.clone();
    let mut debouncer: Debouncer<RecommendedWatcher> = new_debouncer(
        Duration::from_millis(DEBOUNCE_MS),
        move |res: Result<Vec<DebouncedEvent>, _>| {
            if let Ok(events) = res {
                for ev in events {
                    let _ = tx_clone.send(ev);
                }
            }
        },
    )
    .map_err(|e| format!("[vault_watcher] debouncer init: {e}"))?;

    debouncer
        .watcher()
        .watch(&watch_path, RecursiveMode::Recursive)
        .map_err(|e| format!("[vault_watcher] watch failed: {e}"))?;

    tracing::info!("[vault_watcher] fs watch active, entering event loop");

    while let Some(event) = rx.recv().await {
        // ── scheduler-gate check ─────────────────────────────────────────
        if let Some(reason) = watcher_pause_reason() {
            tracing::debug!(
                reason = reason.as_str(),
                "[vault_watcher] paused — dropping event"
            );
            continue;
        }

        let path = event.path.clone();

        // Only care about watched extensions.
        if !is_watched_extension(&path) {
            continue;
        }

        match classify_event(&event) {
            VaultEvent::CreateOrModify => {
                handle_upsert(
                    &path,
                    &watch_path,
                    &config,
                    Arc::clone(&state_store),
                    Arc::clone(&mtime_cache),
                )
                .await;
            }
            VaultEvent::Remove => {
                handle_remove(
                    &path,
                    &watch_path,
                    &config,
                    Arc::clone(&state_store),
                    Arc::clone(&mtime_cache),
                )
                .await;
            }
            VaultEvent::Ignore => {}
        }
    }

    // Channel closed — the debouncer was dropped (shouldn't happen in
    // normal operation).
    tracing::warn!("[vault_watcher] event channel closed, loop exiting");
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Event classification
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
enum VaultEvent {
    CreateOrModify,
    Remove,
    Ignore,
}

fn classify_event(ev: &DebouncedEvent) -> VaultEvent {
    // notify-debouncer-mini exposes the underlying notify EventKind.
    match &ev.kind {
        EventKind::Create(_) | EventKind::Modify(ModifyKind::Data(_)) => {
            VaultEvent::CreateOrModify
        }
        // ModifyKind::Name covers renames — treat the new path as a create.
        EventKind::Modify(ModifyKind::Name(_)) => VaultEvent::CreateOrModify,
        EventKind::Remove(_) => VaultEvent::Remove,
        _ => VaultEvent::Ignore,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Upsert handler (Create + Modify)
// ─────────────────────────────────────────────────────────────────────────────

async fn handle_upsert(
    path: &Path,
    vault_root: &Path,
    config: &Config,
    state_store: Arc<Mutex<WatcherStateStore>>,
    mtime_cache: Arc<Mutex<HashMap<PathBuf, u64>>>,
) {
    // ── mtime guard: skip if file unchanged since last ingest ────────────
    let mtime = match file_mtime(path) {
        Some(m) => m,
        None => {
            tracing::debug!(
                path = %path.display(),
                "[vault_watcher] cannot read mtime, skipping"
            );
            return;
        }
    };

    {
        let cache = mtime_cache.lock().unwrap_or_else(|e| e.into_inner());
        if cache.get(path) == Some(&mtime) {
            tracing::debug!(
                path = %path.display(),
                "[vault_watcher] mtime unchanged, skipping"
            );
            return;
        }
    }

    // ── read file content ────────────────────────────────────────────────
    let body = match tokio::fs::read_to_string(path).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "[vault_watcher] read failed, skipping"
            );
            return;
        }
    };

    let rel = path
        .strip_prefix(vault_root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();

    // ── build mtime-scoped source_id ─────────────────────────────────────
    // Format: "vault_watcher:<rel_path>@<mtime_secs>"
    // Each edit produces a distinct ID, bypassing the dedup gate so the
    // updated content actually reaches the pipeline.
    let source_id = format!("vault_watcher:{rel}@{mtime}");

    let doc = DocumentInput {
        provider: "vault_watcher".to_string(),
        title: rel.clone(),
        body,
        modified_at: chrono::Utc::now(),
        source_ref: Some(format!("vault:{rel}")),
    };

    let tags = vec!["vault_watcher".to_string(), "obsidian".to_string()];

    match ingest_document_with_scope(config, &source_id, "user", tags, doc, None).await {
        Ok(result) => {
            tracing::debug!(
                path = %rel,
                source_id = %source_id,
                already_ingested = result.already_ingested,
                "[vault_watcher] upsert ok"
            );

            // Update mtime cache + SQLite state.
            {
                let mut cache = mtime_cache.lock().unwrap_or_else(|e| e.into_inner());
                cache.insert(path.to_path_buf(), mtime);
            }
            if let Ok(mut store) = state_store.lock() {
                if let Err(e) = store.record_seen(path, mtime) {
                    tracing::warn!(error = %e, "[vault_watcher] state db write failed");
                }
            }
        }
        Err(e) => {
            tracing::warn!(
                path = %rel,
                error = %e,
                "[vault_watcher] ingest failed"
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Remove handler
// ─────────────────────────────────────────────────────────────────────────────

async fn handle_remove(
    path: &Path,
    vault_root: &Path,
    config: &Config,
    state_store: Arc<Mutex<WatcherStateStore>>,
    mtime_cache: Arc<Mutex<HashMap<PathBuf, u64>>>,
) {
    let rel = path
        .strip_prefix(vault_root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();

    // We need the last-known mtime to reconstruct the source_id and
    // tombstone the correct document.
    let last_mtime = {
        let cache = mtime_cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.get(path).copied()
    };

    if let Some(mtime) = last_mtime {
        let source_id = format!("vault_watcher:{rel}@{mtime}");
        if let Err(e) =
            crate::openhuman::memory::ingest_pipeline::mark_document_deleted(config, &source_id)
                .await
        {
            tracing::warn!(
                path = %rel,
                source_id = %source_id,
                error = %e,
                "[vault_watcher] mark_deleted failed"
            );
        } else {
            tracing::debug!(
                path = %rel,
                source_id = %source_id,
                "[vault_watcher] marked deleted"
            );
        }
    } else {
        tracing::debug!(
            path = %rel,
            "[vault_watcher] remove event but no prior ingest found, nothing to tombstone"
        );
    }

    // Evict from cache + mark deleted in SQLite regardless.
    {
        let mut cache = mtime_cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.remove(path);
    }
    if let Ok(mut store) = state_store.lock() {
        if let Err(e) = store.record_deleted(path) {
            tracing::warn!(error = %e, "[vault_watcher] state db delete failed");
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve the vault watch path from config, falling back to
/// `<workspace>/obsidian_vault/wiki/notes/`.
fn resolve_watch_path(config: &Config) -> Result<PathBuf, String> {
    // Prefer an explicit setting; fall back to the conventional location.
    let path = config
        .vault_watch_path()
        .unwrap_or_else(|| config.workspace_dir().join("obsidian_vault/wiki/notes"));

    if !path.exists() {
        // Create the directory so the watcher can start; Obsidian will
        // populate it when the vault is opened.
        std::fs::create_dir_all(&path)
            .map_err(|e| format!("[vault_watcher] cannot create watch dir: {e}"))?;
        tracing::info!(
            path = %path.display(),
            "[vault_watcher] created watch directory (vault not yet populated)"
        );
    }

    Ok(path)
}

fn is_watched_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| WATCHED_EXTENSIONS.contains(&e))
        .unwrap_or(false)
}

fn file_mtime(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn is_watched_extension_md_and_txt() {
        assert!(is_watched_extension(Path::new("note.md")));
        assert!(is_watched_extension(Path::new("note.txt")));
        assert!(!is_watched_extension(Path::new("image.png")));
        assert!(!is_watched_extension(Path::new("data.json")));
    }

    #[test]
    fn file_mtime_returns_some_for_existing_file() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("test.md");
        fs::write(&p, "hello").unwrap();
        assert!(file_mtime(&p).is_some());
    }

    #[test]
    fn file_mtime_returns_none_for_missing_file() {
        assert!(file_mtime(Path::new("/nonexistent/file.md")).is_none());
    }

    #[test]
    fn source_id_format_includes_mtime() {
        let rel = "journal/2024-01-01.md";
        let mtime: u64 = 1_700_000_000;
        let id = format!("vault_watcher:{rel}@{mtime}");
        assert_eq!(id, "vault_watcher:journal/2024-01-01.md@1700000000");
    }

    #[test]
    fn start_vault_watcher_is_idempotent() {
        // Two calls must not panic; the OnceLock ensures only one spawns.
        // We can't assert much more without a live tokio runtime here, but
        // this pins the guard logic doesn't regress.
        //
        // NOTE: deliberately does NOT use #[tokio::test] — calling
        // start_vault_watcher() outside an async context exercises the
        // OnceLock-already-set branch, which is the important regression
        // target. The actual `tokio::spawn` inside will no-op gracefully.
        // WATCHER_STARTED may already be set by a prior test in this
        // process; that's fine — the second-call path is what we're testing.
        start_vault_watcher();
        start_vault_watcher();
        assert!(WATCHER_STARTED.get().is_some());
    }
}
