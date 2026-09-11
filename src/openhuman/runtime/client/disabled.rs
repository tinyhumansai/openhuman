//! The `modules`-off stand-in for the runtime client.
//!
//! Mirrors the surface the real client exposes, answering everything with
//! [`RuntimeCallError::Unavailable`]. A build without the module bus has no way
//! to reach the `tinyruntime` module, and that is a runtime fact rather than a
//! failure: the shell skips its `PATH` injection, and the exec tools are not
//! registered, exactly as when the runtime is disabled in configuration.

use tinyruntime_bus::{ExecResponse, Language, PoolStatsResponse, ResolvedRuntime};

use crate::openhuman::config::Config;

/// Returned by every call in a build without the module bus.
///
/// Phrased as a build fact, matching the `runtime-node` stub's convention.
const MODULES_DISABLED_MESSAGE: &str =
    "the modules feature is disabled at compile time — rebuild with `--features modules` to use \
     managed language runtimes";

/// Why a runtime call did not produce what was asked for.
///
/// The same three variants the real client uses, so callers match identically in
/// both builds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeCallError {
    /// The module is not loaded and cannot be.
    Unavailable(String),
    /// The request was rejected.
    InvalidRequest(String),
    /// Resolution, installation, or execution failed.
    Failed(String),
}

impl std::fmt::Display for RuntimeCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) | Self::InvalidRequest(message) | Self::Failed(message) => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for RuntimeCallError {}

/// The failure every call in this build produces.
fn unavailable() -> RuntimeCallError {
    RuntimeCallError::Unavailable(MODULES_DISABLED_MESSAGE.to_string())
}

/// Always unavailable: there is no module bus to ask.
///
/// # Errors
///
/// Always [`RuntimeCallError::Unavailable`].
pub async fn resolve(
    _config: &Config,
    _language: &Language,
    _install: bool,
) -> Result<Option<ResolvedRuntime>, RuntimeCallError> {
    Err(unavailable())
}

/// Always unavailable: there is no module bus to ask.
///
/// # Errors
///
/// Always [`RuntimeCallError::Unavailable`].
pub async fn execute(
    _config: &Config,
    _language: &Language,
    _code: impl Into<String>,
    _cwd: Option<String>,
    _timeout: Option<std::time::Duration>,
) -> Result<ExecResponse, RuntimeCallError> {
    Err(unavailable())
}

/// Always unavailable: there is no module bus to ask.
///
/// # Errors
///
/// Always [`RuntimeCallError::Unavailable`].
pub async fn pool_stats(_config: &Config) -> Result<PoolStatsResponse, RuntimeCallError> {
    Err(unavailable())
}

#[cfg(test)]
#[path = "disabled_test_tests.rs"]
mod test;
