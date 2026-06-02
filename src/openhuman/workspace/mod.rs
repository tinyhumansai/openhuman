//! Workspace memory-sync module.
//!
//! Responsible for the **user → memory tree** direction of the two-way
//! sync loop:
//!
//! ```text
//! Obsidian vault (wiki/notes/)
//!        │  fs events (notify + debounce)
//!        ▼
//! vault_watcher  ──►  ingest_pipeline  ──►  memory tree
//! ```
//!
//! The agent-→-vault direction (memory tree summaries written to
//! `wiki/` via symlink) is handled by the existing memory-tree pipeline
//! and is not the concern of this module.
//!
//! ## Startup
//!
//! Call [`start`] once during application bootstrap, after the config
//! and scheduler-gate are initialised.  It is idempotent — subsequent
//! calls are no-ops.
//!
//! ```rust,ignore
//! // In bootstrap_core_runtime or equivalent:
//! memory_sync::workspace::start();
//! ```

pub mod watcher;

pub use watcher::start_vault_watcher;

/// Start all workspace sync background tasks.
///
/// Currently starts:
/// - [`watcher::start_vault_watcher`] — watches the Obsidian vault for
///   file-system changes and ingests them into the memory tree.
///
/// Idempotent: safe to call multiple times during startup.
pub fn start() {
    start_vault_watcher();
}
