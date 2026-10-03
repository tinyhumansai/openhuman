//! Where one agent's files live.
//!
//! Everything an agent owns sits under the runtime's workspace, keyed by the
//! agent id:
//!
//! ```text
//! <root>/
//!   config.toml, auth-profiles.json, core.token      runtime-wide
//!   workspace/
//!     session_db/sessions.db                         runtime-wide run ledger
//!     agents/<id>/{SOUL.md, skills/}                     the agent's home
//!     session_raw/<ts>_<id>.jsonl                    its transcripts
//!   agents/<id>/action/                              its default action_dir
//! ```

use std::path::{Path, PathBuf};

/// Resolved per-agent paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLayout {
    /// `<workspace>/agents/<id>/`.
    pub home: PathBuf,
    /// `<workspace>/agents/<id>/skills/`.
    pub skills: PathBuf,
    /// The transcript directory the harness writes for this agent.
    pub transcripts: PathBuf,
    /// The agent's read/write root.
    pub action_dir: PathBuf,
}

impl AgentLayout {
    /// Lay out `id` under `workspace_dir`, with `action_dir` either the
    /// spec's explicit choice or the default for this runtime's workspace
    /// kind.
    pub(crate) fn resolve(workspace_dir: &Path, id: &str, action_dir: PathBuf) -> Self {
        Self {
            home: workspace_dir.join("agents").join(id),
            skills: workspace_dir.join("agents").join(id).join("skills"),
            transcripts: workspace_dir.join("session_raw"),
            action_dir,
        }
    }

    /// The default `action_dir` for agent `id`.
    ///
    /// Runtime-owned roots get `<root>/agents/<id>/action` — a sibling of the
    /// workspace, never inside it, because `is_workspace_internal_path`
    /// blocks agent writes beneath the workspace fail-closed. An inherited
    /// workspace gets `<action_dir>/agents/<id>`.
    pub(crate) fn default_action_dir(
        root_dir: &Path,
        runtime_action_dir: &Path,
        inherited: bool,
        id: &str,
    ) -> PathBuf {
        if inherited {
            runtime_action_dir.join("agents").join(id)
        } else {
            root_dir.join("agents").join(id).join("action")
        }
    }
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
