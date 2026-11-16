//! GitHub-release update staging: find the newest release of a binary, pick
//! this platform's asset, and download, verify and stage it.
//!
//! The caller names the release with an [`UpdateSource`] and reports failures
//! through an [`UpdateReporter`]. Which releases may be applied, whether a
//! caller may trigger one, and how the process restarts afterwards are the
//! caller's policy, not this module's.

mod download;
mod release;
mod types;

pub use release::{check_available, download_and_stage, platform_triple};
pub use types::{
    GitHubAsset, GitHubRelease, StagedUpdate, UpdateInfo, UpdateReporter, UpdateSource,
};

#[cfg(test)]
mod test_support;
