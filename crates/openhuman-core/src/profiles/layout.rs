//! Where one profile's state lives, and the config it always runs with.
//!
//! ```text
//! <root>/users/<profile-id>/
//!   profile.toml      ProfileMeta
//!   config.toml       the profile's config_path
//!   workspace/        sessions, memory, threads, cron, cost — internal state
//!   sandbox/          the profile's action_dir: the only place it may act
//! <root>/deprovisioned/<profile-id>-<unix-secs>-<uuid>/   an archived profile
//! ```
//!
//! The `users/<id>` shape is the desktop's user directory; both build it
//! through the shared [`ProfileLayout`]. Every per-profile path sits under its
//! own directory, so each store keyed by workspace (cron, approvals, threads,
//! the cost ledger, the session store) is already separate per user without
//! that store knowing about users.

use std::path::{Path, PathBuf};

use super::types::ProfileId;
use crate::config::schema::MemoryLayoutMode;
use crate::config::Config;
use crate::security::AutonomyLevel;

pub use crate::config::schema::profile_layout::{users_dir, ProfileLayout};

/// The layout of profile `id` under the SaaS root.
pub fn profile_layout(saas_root: &Path, id: &ProfileId) -> ProfileLayout {
    ProfileLayout::new(saas_root, id)
}

/// `<root>/deprovisioned`.
pub fn archive_dir(saas_root: &Path) -> PathBuf {
    saas_root.join("deprovisioned")
}

/// The memory namespace root of profile `id`.
pub fn memory_root(id: &ProfileId) -> String {
    format!("user:{id}")
}

/// The config profile `id` runs with.
///
/// Everything that decides **where** the profile reads and writes, and **what
/// it may do**, is forced here and cannot come from anywhere else:
///
/// - every path sits under the profile's own directory;
/// - memory is bound to the profile (`[memory] agent_id`, `root = user:<id>`),
///   so a definition pin or a team root cannot move it onto another user's
///   tree (a host binding wins over both);
/// - the memory layout is pinned to the legacy tree, confined to that root
///   by `tinymemory_tools::confine::Policy::Clamp`, selected by
///   `memory::ops`. Layout v3 would ignore `root` and bind the engine below
///   `memory::scope::user_root`, which reads the person from the config
///   path (`users/<id>/config.toml`): `org:<id>` for an id that looks like a
///   TinyHumans account, a freshly minted `org:local-…` otherwise. Either
///   disagrees with `user:<id>`, so a profile never runs v3;
/// - the autonomy policy is on and supervised, with no auto-approval, no tool
///   installation, no trusted roots beyond the sandbox, and workspace-only
///   paths.
pub fn profile_config(layout: &ProfileLayout, id: &ProfileId) -> Config {
    let mut config = Config {
        config_path: layout.config_path.clone(),
        workspace_dir: layout.workspace_dir.clone(),
        action_dir: layout.sandbox_dir.clone(),
        ..Config::default()
    };
    // Artifacts land in the profile's sandbox, never the host's shared
    // `~/OpenHuman/projects/Files`.
    config.files_dir_override = Some(layout.sandbox_dir.join("files"));
    config.memory.agent_id = Some(id.to_string());
    config.memory.root = Some(memory_root(id));
    config.memory.layout = MemoryLayoutMode::Legacy;
    config.agent.channel_permissions =
        std::collections::HashMap::from([("profile".into(), "readonly".into())]);

    let autonomy = &mut config.autonomy;
    autonomy.enabled = true;
    autonomy.level = AutonomyLevel::Supervised;
    autonomy.workspace_only = true;
    autonomy.auto_approve_all = false;
    autonomy.allow_tool_install = false;
    autonomy.trusted_roots = Vec::new();
    config
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
