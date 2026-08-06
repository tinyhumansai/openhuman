//! Persistence for the long-term goals list — thin host shim over
//! `tinycortex::memory::goals::store` (W7).
//!
//! The engine (read / write / mutate / cap of `MEMORY_GOALS.md`) is the crate's.
//! These wrappers keep the host's `async` + `Result<_, String>` signatures so
//! the RPC ops, agent tools, and the reflection (`enrich`) caller are unchanged.
//! On-disk layout is identical: `<workspace_dir>/MEMORY_GOALS.md` in the
//! workspace root (`GOALS_FILE`), with the same render/parse format.

use std::path::{Path, PathBuf};

use tinycortex::memory::goals::store as engine;
use tinycortex::memory::goals::types::GoalsDoc;

pub use engine::{GOALS_FILE, GOALS_FILE_MAX_CHARS, GOALS_MAX_ITEMS};

/// Absolute path to `MEMORY_GOALS.md` within `workspace_dir`.
pub fn goals_path(workspace_dir: &Path) -> PathBuf {
    engine::goals_path(workspace_dir)
}

/// Load the goals document (a missing file maps to an empty doc).
pub async fn load(workspace_dir: &Path) -> Result<GoalsDoc, String> {
    engine::load(workspace_dir).map_err(|e| e.to_string())
}

/// Persist the goals document, enforcing the item/char caps.
pub async fn save(workspace_dir: &Path, doc: &mut GoalsDoc) -> Result<(), String> {
    engine::save(workspace_dir, doc).map_err(|e| e.to_string())
}

/// Append a goal; returns the new item's id and the updated doc.
pub async fn add(workspace_dir: &Path, text: &str) -> Result<(String, GoalsDoc), String> {
    engine::add(workspace_dir, text).map_err(|e| e.to_string())
}

/// Edit an existing goal by id.
pub async fn edit(workspace_dir: &Path, id: &str, text: &str) -> Result<GoalsDoc, String> {
    engine::edit(workspace_dir, id, text).map_err(|e| e.to_string())
}

/// Delete a goal by id.
pub async fn delete(workspace_dir: &Path, id: &str) -> Result<GoalsDoc, String> {
    engine::delete(workspace_dir, id).map_err(|e| e.to_string())
}
