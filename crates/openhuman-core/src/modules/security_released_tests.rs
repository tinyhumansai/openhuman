//! Test support: a [`SecurityPolicy`] whose shell and class questions are
//! answered by the released TinySecurity archive through the production client
//! (no gateway stub), for the ignored tests that drive a moved caller end to
//! end. The archive is downloaded into a scratch install dir under `target/tmp`
//! (`TMPDIR` under `/tmp` breaks the path-policy tests).

use crate::security::{AutonomyLevel, SecurityPolicy};
use std::path::PathBuf;

/// A scratch install, its workspace and action directories. Keep it alive for
/// as long as its policies are used.
pub(crate) struct Released {
    _dir: tempfile::TempDir,
    pub(crate) workspace: PathBuf,
    pub(crate) action: PathBuf,
}

impl Released {
    pub(crate) fn new() -> Self {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
        std::fs::create_dir_all(&root).unwrap();
        let dir = tempfile::tempdir_in(&root).unwrap();
        let base = dir.path().canonicalize().unwrap();
        let workspace = base.join("workspace");
        let action = base.join("action");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&action).unwrap();
        std::fs::write(
            base.join("config.toml"),
            format!(
                "[modules]\nenabled = true\nallow_download = true\ninstall_dir = {:?}\n",
                base.join("modules").to_string_lossy()
            ),
        )
        .unwrap();
        Self {
            _dir: dir,
            workspace,
            action,
        }
    }

    /// An enabled policy at `autonomy` with the module as its only gate.
    pub(crate) fn policy(
        &self,
        autonomy: AutonomyLevel,
        max_actions_per_hour: u32,
    ) -> SecurityPolicy {
        SecurityPolicy {
            enabled: true,
            autonomy,
            workspace_dir: self.workspace.clone(),
            action_dir: self.action.clone(),
            max_actions_per_hour,
            ..SecurityPolicy::default()
        }
    }
}
