//! The core's release source: binds `openhuman-platform`'s GitHub-release
//! staging to the core binary's releases, its version, and Sentry.

use std::path::PathBuf;

use openhuman_platform::update::{self as staging, UpdateInfo, UpdateReporter, UpdateSource};

use crate::config::UpdateRestartStrategy;
use crate::platform::update::types::UpdateApplyResult;

/// Where `openhuman-core` releases are published. Compiled in on purpose: see
/// [`UpdateSource`].
pub(crate) const CORE_RELEASES: UpdateSource = UpdateSource {
    owner: "tinyhumansai",
    repo: "openhuman",
    asset_prefix: "openhuman-core",
};

/// Reports update failures to Sentry under the `update` domain, skipping the
/// updater's known-transient transport messages and HTTP statuses.
struct SentryReporter;

impl UpdateReporter for SentryReporter {
    fn report(&self, message: &str, operation: &str, tags: &[(&str, &str)]) {
        crate::core::observability::report_error(message, "update", operation, tags);
    }

    fn is_transient_message(&self, message: &str) -> bool {
        crate::core::observability::is_updater_transient_message(message)
    }

    fn is_transient_http_status(&self, status: u16) -> bool {
        crate::core::observability::is_updater_transient_http_status(status)
    }
}

/// Current binary version (set at compile time from Cargo.toml).
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Check GitHub Releases for a newer version of openhuman-core.
pub async fn check_available() -> Result<UpdateInfo, String> {
    staging::check_available(&CORE_RELEASES, current_version(), &SentryReporter).await
}

/// Download, verify and stage the updated binary.
///
/// The caller (Tauri shell, or the self-restart under `self_replace`) is
/// responsible for restarting with the new binary. `staging_dir` of `None`
/// stages next to the running executable.
pub async fn download_and_stage(
    download_url: &str,
    asset_name: &str,
    staging_dir: Option<PathBuf>,
) -> Result<UpdateApplyResult, String> {
    download_and_stage_with_version(download_url, asset_name, staging_dir, None, None).await
}

/// `download_and_stage`, with the release metadata the caller already holds.
///
/// `target_version` — the version of the release being staged, used in the
/// returned `UpdateApplyResult`. If `None`, falls back to `current_version()`.
///
/// `expected_size` — the asset size the release declared. The body must be
/// exactly this long. If `None`, the body is capped at the staging default.
pub async fn download_and_stage_with_version(
    download_url: &str,
    asset_name: &str,
    staging_dir: Option<PathBuf>,
    target_version: Option<&str>,
    expected_size: Option<u64>,
) -> Result<UpdateApplyResult, String> {
    let staged = staging::download_and_stage(
        &CORE_RELEASES,
        download_url,
        asset_name,
        staging_dir,
        expected_size,
        &SentryReporter,
    )
    .await?;

    Ok(UpdateApplyResult {
        installed_version: target_version.unwrap_or(current_version()).to_string(),
        staged_path: staged.staged_path.to_string_lossy().to_string(),
        restart_required: true,
        restart_strategy: UpdateRestartStrategy::SelfReplace,
    })
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
