//! Types for the self-update domain.
//!
//! Release discovery types (`UpdateInfo`, the GitHub release shapes) live in
//! `openhuman_platform::update`.

use serde::{Deserialize, Serialize};

use crate::config::UpdateRestartStrategy;

/// Lightweight identity of the running core binary, returned by
/// `update.version`. Lets the frontend decide whether to call
/// `update.check` / `update.run` without paying the GitHub round-trip
/// just to discover what version it is talking to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    /// Current binary version (`CARGO_PKG_VERSION`).
    pub version: String,
    /// Rust target triple this binary was built for.
    pub target_triple: String,
    /// The asset name prefix used by the GitHub release flow
    /// (`openhuman-core-{target_triple}`). Frontends can match against
    /// this to find a compatible asset without re-deriving the triple.
    pub asset_prefix: String,
}

/// Outcome of the orchestrated `update.run` flow (check → apply →
/// restart). Keeps every interesting field flat so the frontend can
/// decide what to surface to the user without re-walking the response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateRunResult {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    /// True when a new binary was successfully downloaded + staged.
    pub applied: bool,
    /// Set when `applied` is true.
    pub staged_path: Option<String>,
    /// True when a self-restart was published. The process will exit
    /// shortly after the RPC response is returned.
    pub restart_requested: bool,
    /// The configured post-stage restart contract.
    pub restart_strategy: UpdateRestartStrategy,
    /// Human-readable summary suitable for logs / surface text.
    pub message: String,
}

/// Result of applying an update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateApplyResult {
    /// The version that was installed.
    pub installed_version: String,
    /// Path where the new binary was staged.
    pub staged_path: String,
    /// Whether a restart is required to complete the update.
    pub restart_required: bool,
    /// The configured post-stage restart contract.
    pub restart_strategy: UpdateRestartStrategy,
}
