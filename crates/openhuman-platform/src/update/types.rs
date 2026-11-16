//! Inputs and results of release discovery and staging.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Where a binary's releases are published and how its assets are named.
///
/// The fields are `&'static str` on purpose: the release origin is compiled
/// in, never read from the environment or config, because a runtime-settable
/// source would let anything that can set it redirect a self-update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateSource {
    /// GitHub owner of the release repository.
    pub owner: &'static str,
    /// GitHub repository holding the releases.
    pub repo: &'static str,
    /// Stem of the binary and its assets, e.g. `openhuman-core`. Release
    /// archives are `<prefix>-<version>-<triple>.tar.gz` (`.zip` on Windows),
    /// legacy raw binaries `<prefix>-<triple>`, and the archive holds the
    /// executable as `<prefix>` (`<prefix>.exe` on Windows).
    pub asset_prefix: &'static str,
}

/// How staging reports failures. The crate logs locally either way; an
/// implementation decides what reaches an error tracker.
pub trait UpdateReporter: Sync {
    /// Report a failure worth an error event. `operation` is
    /// `check_releases` or `download`; `tags` are low-cardinality pairs.
    fn report(&self, message: &str, operation: &str, tags: &[(&str, &str)]);

    /// True when a transport-failure message is known noise (offline,
    /// blocked host) that should be logged but not reported.
    fn is_transient_message(&self, _message: &str) -> bool {
        false
    }

    /// True when an HTTP status from the release host is known noise (rate
    /// limits, upstream outages) that should be logged but not reported.
    fn is_transient_http_status(&self, _status: u16) -> bool {
        false
    }
}

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
    /// Asset size in bytes, as the release declared it. The download is
    /// refused unless its body is exactly this long.
    #[serde(default)]
    pub asset_size: Option<u64>,
    /// Release notes / body from GitHub.
    pub release_notes: Option<String>,
    /// When the release was published (ISO 8601).
    pub published_at: Option<String>,
}

/// A verified binary written to its staging path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedUpdate {
    /// Where the new executable now is.
    pub staged_path: PathBuf,
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
