//! Where the harness keeps its state.
//!
//! Session history, memory, attachments and skills all live beneath one
//! directory, and `tinyagents::session` keys its SQLite database on it
//! (`{workspace}/session_db/sessions.db`). There is no storage trait to swap and
//! the in-memory database variant is `#[cfg(test)]`, so **the workspace
//! directory is the only storage axis an embedder has** — which is precisely why
//! it is a first-class builder input rather than something discovered from the
//! environment.

use std::path::{Path, PathBuf};

use super::error::HarnessError;

/// Which workspace an embedded harness runs against.
#[derive(Debug, Clone, Default)]
pub enum Workspace {
    /// A throwaway directory, removed when the [`Harness`](super::Harness) is
    /// dropped.
    ///
    /// The default, and deliberately so: a library call that silently wrote
    /// into the operator's real install would make "try this out" a destructive
    /// act. Sessions do not survive the process — pass [`Workspace::Dir`] when
    /// they should.
    #[default]
    Ephemeral,
    /// A caller-owned directory. Created if absent; persists across runs.
    Dir(PathBuf),
    /// The machine's configured OpenHuman workspace — the one the desktop app
    /// and CLI use, resolved the usual way (`OPENHUMAN_WORKSPACE`,
    /// `active_user.toml`, `~/.openhuman/...`).
    ///
    /// Reuses an existing signed-in install, and equally can disturb it. The
    /// harness will not copy skills into an inherited workspace for that reason.
    Inherit,
}

impl Workspace {
    /// A throwaway workspace under the system temp dir.
    pub fn ephemeral() -> Self {
        Self::Ephemeral
    }

    /// A caller-owned workspace directory.
    pub fn dir(path: impl Into<PathBuf>) -> Self {
        Self::Dir(path.into())
    }

    /// Whether this workspace belongs to the operator rather than the harness.
    ///
    /// `true` only for [`Workspace::Inherit`]. Guards the writes the harness
    /// would otherwise make on the caller's behalf — see
    /// [`skills`](super::skills).
    pub fn is_operator_owned(&self) -> bool {
        matches!(self, Self::Inherit)
    }
}

/// A resolved workspace: concrete paths plus, for [`Workspace::Ephemeral`], the
/// [`tempfile::TempDir`] whose lifetime owns them.
///
/// The `TempDir` is held rather than leaked so the directory is removed on drop.
/// Losing it would turn every ephemeral harness into a permanent temp-dir leak,
/// which on a machine whose `/tmp` is a tmpfs is a RAM leak.
pub(super) struct ResolvedWorkspace {
    pub(super) workspace_dir: PathBuf,
    pub(super) action_dir: PathBuf,
    /// Where `config.toml` would live. **Not** cosmetic: its *parent* is the
    /// state directory the credential store, the auth profiles and the keyring
    /// file backend all resolve against. Leave it at the default while pointing
    /// `workspace_dir` at a temp dir and the harness reads and writes the
    /// operator's real `~/.openhuman` credentials while looking hermetic.
    pub(super) config_path: PathBuf,
    /// `None` for `Dir` / `Inherit`; those directories outlive the harness.
    pub(super) _temp: Option<tempfile::TempDir>,
}

impl ResolvedWorkspace {
    /// Materialize `workspace`, creating directories as needed.
    ///
    /// The layout for the harness-owned variants mirrors
    /// `src/bin/library_profile/harness.rs::fixture()`, which is the recipe
    /// already proven against real turns: a `workspace/` for internal state and
    /// a sibling `action/` for the agent's read/write root. Keeping them
    /// siblings rather than nesting matters — `is_workspace_internal_path`
    /// blocks agent writes beneath the workspace fail-closed, so an `action_dir`
    /// inside it would be an agent that cannot write anywhere.
    ///
    /// `config.toml` sits beside the workspace rather than inside it, which is
    /// the same shape `load_or_init` produces (`<root>/config.toml` next to
    /// `<root>/workspace`). Credential state follows that file's parent, so the
    /// layout is what makes an ephemeral harness actually ephemeral.
    pub(super) fn resolve(
        workspace: &Workspace,
        action_dir_override: Option<&Path>,
    ) -> Result<Self, HarnessError> {
        match workspace {
            Workspace::Ephemeral => {
                let temp = tempfile::Builder::new()
                    .prefix("openhuman-harness-")
                    .tempdir()
                    .map_err(|source| HarnessError::Workspace {
                        what: "create a temporary workspace",
                        source,
                    })?;
                let root = temp.path().to_path_buf();
                let workspace_dir = root.join("workspace");
                let action_dir = action_dir_override
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| root.join("action"));
                create_dirs(&workspace_dir, &action_dir)?;
                log::debug!("[embed][harness] ephemeral workspace at {}", root.display());
                Ok(Self {
                    workspace_dir,
                    action_dir,
                    config_path: root.join("config.toml"),
                    _temp: Some(temp),
                })
            }
            Workspace::Dir(dir) => {
                let workspace_dir = dir.clone();
                // `Path::parent()` yields `Some("")` for a single-component
                // relative path (e.g. `Workspace::dir("ws")`), not `None`.
                // Treating that empty parent as a real directory would make the
                // sibling `action/` and `config.toml` resolve against the
                // process working directory instead of beside the workspace,
                // which breaks the credential-isolation invariant below.
                let parent = dir.parent().filter(|parent| !parent.as_os_str().is_empty());
                let action_dir = action_dir_override
                    .map(Path::to_path_buf)
                    // A sibling, for the `is_workspace_internal_path` reason
                    // above. `dir.parent()` is `None` (or an empty path) only
                    // for a filesystem root or a path with no parent — all
                    // cases where a sibling is meaningless anyway, and using
                    // the workspace dir itself is the only safe fallback.
                    .or_else(|| parent.map(|parent| parent.join("action")))
                    .unwrap_or_else(|| dir.join("action"));
                create_dirs(&workspace_dir, &action_dir)?;
                let config_path = parent.unwrap_or(dir.as_path()).join("config.toml");
                Ok(Self {
                    workspace_dir,
                    action_dir,
                    config_path,
                    _temp: None,
                })
            }
            Workspace::Inherit => {
                // Resolved by `Config::load_or_init` rather than here: the
                // chain it walks (OPENHUMAN_WORKSPACE, active_user.toml, the
                // per-user `users/` scoping) is the operator's, and
                // re-deriving it would be a second implementation that drifts.
                // The builder signals this by not overriding the loaded fields.
                Ok(Self {
                    workspace_dir: PathBuf::new(),
                    action_dir: action_dir_override
                        .map(Path::to_path_buf)
                        .unwrap_or_default(),
                    config_path: PathBuf::new(),
                    _temp: None,
                })
            }
        }
    }
}

fn create_dirs(workspace_dir: &Path, action_dir: &Path) -> Result<(), HarnessError> {
    for dir in [workspace_dir, action_dir] {
        std::fs::create_dir_all(dir).map_err(|source| HarnessError::Workspace {
            what: "create the workspace directory",
            source,
        })?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
