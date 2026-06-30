//! Persistence for the long-term goals list.
//!
//! The goals list lives in a single compact markdown file,
//! `MEMORY_GOALS.md`, in the user's `workspace_dir` (the same root as
//! `MEMORY.md` / `PROFILE.md`). The file is intentionally tiny — capped at
//! ~500 tokens — so it stays cheap to read and easy for a human to edit.
//!
//! All mutations go through load → mutate → [`save`], which re-enforces the
//! size + item-count caps on every write. Trimming drops the *oldest* items
//! first (front of the list) and logs what it removed; it never silently
//! truncates.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tokio::sync::Mutex;

use super::types::GoalsDoc;

/// Serialises load→mutate→save sequences so concurrent callers (user edits via
/// RPC/tools and background `spawn_enrich_goals`) can't clobber each other.
fn goals_mutation_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// File name of the goals document inside `workspace_dir`.
pub const GOALS_FILE: &str = "MEMORY_GOALS.md";

/// Hard ceiling on the rendered file size. ~2000 chars ≈ ~500 tokens, which
/// keeps the document inside the "200–500 token" budget from the spec.
pub const GOALS_FILE_MAX_CHARS: usize = 2000;

/// Maximum number of goal items. A long-term goals list should be short and
/// focused; beyond this we drop the oldest entries.
pub const GOALS_MAX_ITEMS: usize = 8;

/// Absolute path to `MEMORY_GOALS.md` within `workspace_dir`.
pub fn goals_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join(GOALS_FILE)
}

/// Verify that the resolved goals path stays inside `workspace_dir`,
/// defending against symlink-based escapes. Returns the validated path.
fn validate_within_workspace(workspace_dir: &Path) -> Result<PathBuf, String> {
    let path = goals_path(workspace_dir);
    // Canonicalize the parent (the workspace) — the file itself may not
    // exist yet on first write.
    let workspace_canon = workspace_dir
        .canonicalize()
        .unwrap_or_else(|_| workspace_dir.to_path_buf());
    let parent = path.parent().unwrap_or(workspace_dir);
    let parent_canon = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf());
    if !parent_canon.starts_with(&workspace_canon) {
        return Err(format!(
            "[memory_goals] goals path resolves outside workspace: {path:?}"
        ));
    }
    // If the file already exists as a symlink, ensure its target also stays
    // inside the workspace — a symlinked MEMORY_GOALS.md could otherwise
    // read/write outside the boundary even with a valid parent dir.
    if let Ok(meta) = std::fs::symlink_metadata(&path) {
        if meta.file_type().is_symlink() {
            let resolved = path.canonicalize().map_err(|e| {
                format!("[memory_goals] failed to resolve goals symlink {path:?}: {e}")
            })?;
            if !resolved.starts_with(&workspace_canon) {
                return Err(format!(
                    "[memory_goals] goals symlink resolves outside workspace: {resolved:?}"
                ));
            }
        }
    }
    Ok(path)
}

/// Load the goals document from disk. Returns an empty document when the
/// file does not exist yet (first run).
pub async fn load(workspace_dir: &Path) -> Result<GoalsDoc, String> {
    let path = validate_within_workspace(workspace_dir)?;
    match tokio::fs::read_to_string(&path).await {
        Ok(body) => {
            let doc = GoalsDoc::parse(&body);
            log::debug!(
                "[memory_goals] loaded {} item(s) from {path:?}",
                doc.items.len()
            );
            Ok(doc)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::debug!("[memory_goals] no goals file at {path:?} — starting empty");
            Ok(GoalsDoc::default())
        }
        Err(e) => Err(format!("[memory_goals] failed to read {path:?}: {e}")),
    }
}

/// Enforce the item-count and byte-size caps on `doc`, dropping the oldest
/// items as needed. Returns the list of dropped item ids (for logging).
fn enforce_caps(doc: &mut GoalsDoc) -> Vec<String> {
    let mut dropped = Vec::new();
    // 1. Item-count cap.
    while doc.items.len() > GOALS_MAX_ITEMS {
        let removed = doc.items.remove(0);
        dropped.push(removed.id);
    }
    // 2. Byte-size cap — keep removing the oldest until the rendered file
    //    fits. Always leave at least one item if any remain so the file
    //    isn't pointlessly emptied by a single oversized entry.
    while doc.render().len() > GOALS_FILE_MAX_CHARS && doc.items.len() > 1 {
        let removed = doc.items.remove(0);
        dropped.push(removed.id);
    }
    dropped
}

/// Persist `doc` to disk, enforcing caps first. The `doc` is mutated in
/// place to reflect any cap trimming so the caller's view matches disk.
pub async fn save(workspace_dir: &Path, doc: &mut GoalsDoc) -> Result<(), String> {
    let path = validate_within_workspace(workspace_dir)?;

    let dropped = enforce_caps(doc);
    if !dropped.is_empty() {
        log::warn!(
            "[memory_goals] dropped {} oldest item(s) to fit caps (max_items={}, max_chars={}): {:?}",
            dropped.len(),
            GOALS_MAX_ITEMS,
            GOALS_FILE_MAX_CHARS,
            dropped
        );
    }

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("[memory_goals] failed to create {parent:?}: {e}"))?;
    }

    let body = doc.render();
    tokio::fs::write(&path, &body)
        .await
        .map_err(|e| format!("[memory_goals] failed to write {path:?}: {e}"))?;
    log::debug!(
        "[memory_goals] saved {} item(s) ({} bytes) to {path:?}",
        doc.items.len(),
        body.len()
    );
    Ok(())
}

/// Add a goal, persist, and return `(new_id, updated_doc)`.
pub async fn add(workspace_dir: &Path, text: &str) -> Result<(String, GoalsDoc), String> {
    let _guard = goals_mutation_lock().lock().await;
    let mut doc = load(workspace_dir).await?;
    let id = doc.add(text)?;
    save(workspace_dir, &mut doc).await?;
    log::info!("[memory_goals] added goal id={id}");
    Ok((id, doc))
}

/// Edit a goal's text, persist, and return the updated document.
pub async fn edit(workspace_dir: &Path, id: &str, text: &str) -> Result<GoalsDoc, String> {
    let _guard = goals_mutation_lock().lock().await;
    let mut doc = load(workspace_dir).await?;
    doc.edit(id, text)?;
    save(workspace_dir, &mut doc).await?;
    log::info!("[memory_goals] edited goal id={id}");
    Ok(doc)
}

/// Delete a goal, persist, and return the updated document.
pub async fn delete(workspace_dir: &Path, id: &str) -> Result<GoalsDoc, String> {
    let _guard = goals_mutation_lock().lock().await;
    let mut doc = load(workspace_dir).await?;
    doc.delete(id)?;
    save(workspace_dir, &mut doc).await?;
    log::info!("[memory_goals] deleted goal id={id}");
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn load_empty_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let doc = load(tmp.path()).await.unwrap();
        assert!(doc.is_empty());
    }

    #[tokio::test]
    async fn add_edit_delete_round_trip_to_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let (id, _) = add(tmp.path(), "ship the app").await.unwrap();

        let reloaded = load(tmp.path()).await.unwrap();
        assert_eq!(reloaded.items.len(), 1);
        assert_eq!(reloaded.items[0].text, "ship the app");

        edit(tmp.path(), &id, "ship the app to all platforms")
            .await
            .unwrap();
        let reloaded = load(tmp.path()).await.unwrap();
        assert_eq!(reloaded.items[0].text, "ship the app to all platforms");

        delete(tmp.path(), &id).await.unwrap();
        let reloaded = load(tmp.path()).await.unwrap();
        assert!(reloaded.is_empty());
    }

    #[tokio::test]
    async fn save_enforces_item_count_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let mut doc = GoalsDoc::default();
        for i in 0..(GOALS_MAX_ITEMS + 3) {
            doc.add(&format!("goal number {i}")).unwrap();
        }
        save(tmp.path(), &mut doc).await.unwrap();
        assert_eq!(doc.items.len(), GOALS_MAX_ITEMS);
        // The oldest items (goal number 0..2) should have been dropped.
        assert!(doc.items.iter().all(|i| i.text != "goal number 0"));
    }

    #[tokio::test]
    async fn save_enforces_byte_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let mut doc = GoalsDoc::default();
        // Two large items that together exceed the byte cap.
        let big = "x".repeat(GOALS_FILE_MAX_CHARS);
        doc.add(&big).unwrap();
        doc.add(&big).unwrap();
        save(tmp.path(), &mut doc).await.unwrap();
        // At least one item dropped; never fully emptied.
        assert_eq!(doc.items.len(), 1);
    }
}
