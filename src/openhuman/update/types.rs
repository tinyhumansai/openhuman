//! Types for the self-update domain.

use serde::{Deserialize, Serialize};

/// Summary of an available update from GitHub Releases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    /// The latest version tag (e.g. "0.50.0").
    pub latest_version: String,
    /// The currently running version.
    pub current_version: String,
    /// Whether an update is available (`latest_version > current_version`).
    pub update_available: bool,
    /// Direct download URL for the platform-appropriate asset.
    pub download_url: Option<String>,
    /// Asset file name.
    pub asset_name: Option<String>,
    /// Release notes / body from GitHub.
    pub release_notes: Option<String>,
    /// When the release was published (ISO 8601).
    pub published_at: Option<String>,
}

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
}

/// Subset of the GitHub Releases API response we care about.
#[derive(Debug, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    pub body: Option<String>,
    pub published_at: Option<String>,
    pub assets: Vec<GitHubAsset>,
}

/// A single asset attached to a GitHub release.
#[derive(Debug, Deserialize)]
pub struct GitHubAsset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
}
