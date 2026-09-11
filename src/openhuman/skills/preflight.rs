//! Workflow preflight gates — run BEFORE the orchestrator boots for a
//! `skills_run`, so failures surface as a plain `Err` from
//! [`crate::openhuman::skills::runtime::spawn_workflow_run_background`] (and from there into
//! the dashboard card / runner page UI) instead of leaking through as
//! cryptic orchestrator output.
//!
//! The first gate this module ships is the **GitHub gate**: when a
//! skill's `[github]` block sets `required = true`, we assert that
//!
//! 1. The Composio GitHub integration is connected (`toolkit ==
//!    "github"` AND `is_active() == true`).
//! 2. Local `git` is on PATH (`git --version` exits zero).
//! 3. `git config --global user.name` AND `git config --global
//!    user.email` are both set to non-empty trimmed values.
//! 4. When `identity_match == Strict`, the Composio GitHub username
//!    equals `git config user.name` case-insensitively.
//!
//! Each check has its own [`GithubGateError`] variant carrying enough
//! context for a user-readable explanation (which check failed, what
//! the remediation is). The gate decision is also serialised into the
//! run-log header by the caller so failures appear in the existing
//! in-app log viewer.
//!
//! ## Testability
//!
//! The gate is built around a [`PreflightProbes`] trait, so unit tests
//! can swap a stub probe in for the four side effects (Composio
//! lookup, `git --version`, `git config user.name`, `git config
//! user.email`) without spinning up a real git binary or a live
//! Composio connection. This matches the established
//! "inject-the-async-closure" pattern used elsewhere in the skills
//! domain (see e.g. `registry.rs`'s `cfg(test)` indirections) — no new
//! mocking framework is introduced.

use std::time::Duration;

use async_trait::async_trait;

use crate::openhuman::config::Config;
use crate::openhuman::integrations::composio::{self};

use super::registry::{IdentityMatch, WorkflowGithubConfig};

/// Hard cap on each local `git` subprocess probe so a wedged git
/// (e.g. credential prompt, hung filesystem) can't stall the preflight
/// gate indefinitely.
const GIT_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// One reason the GitHub preflight gate refused to start a skill_run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubGateError {
    /// No active Composio connection found whose `toolkit == "github"`.
    ComposioGithubMissing,
    /// `git --version` failed (binary missing, non-zero exit, …). The
    /// payload carries the underlying error message for the run log.
    GitBinaryMissing(String),
    /// `git config --global user.name` was empty / unset.
    GitUserNameMissing,
    /// `git config --global user.email` was empty / unset.
    GitUserEmailMissing,
    /// `identity_match = "strict"` was on, both names existed, but
    /// `local != composio` (case-insensitive after trim).
    IdentityMismatch {
        composio_username: String,
        git_username: String,
    },
    /// `identity_match = "strict"` was on but the Composio side
    /// couldn't resolve a username (no profile reachable). Distinct
    /// from `ComposioGithubMissing` because the connection IS there —
    /// the identity lookup just failed.
    ComposioIdentityUnresolved,
}

impl GithubGateError {
    /// Render the gate failure as a single user-readable string with
    /// concrete remediation. This is what `spawn_workflow_run_background`
    /// puts in the `Err(String)` it returns.
    pub fn to_user_message(&self, log_path: Option<&str>) -> String {
        let body = match self {
            GithubGateError::ComposioGithubMissing => {
                "GitHub preflight failed: no active Composio GitHub connection. \
                 Connect via `composio_authorize github` (or Connections → \
                 OAuth → GitHub) and re-run."
                    .to_string()
            }
            GithubGateError::GitBinaryMissing(err) => format!(
                "GitHub preflight failed: local `git` is not available ({err}). \
                 Install Git and make sure it's on PATH, then re-run."
            ),
            GithubGateError::GitUserNameMissing => {
                "GitHub preflight failed: `git config --global user.name` is empty. \
                 Run `git config --global user.name \"<your name>\"` and re-run."
                    .to_string()
            }
            GithubGateError::GitUserEmailMissing => {
                "GitHub preflight failed: `git config --global user.email` is empty. \
                 Run `git config --global user.email \"<you@example.com>\"` and re-run."
                    .to_string()
            }
            GithubGateError::IdentityMismatch {
                composio_username,
                git_username,
            } => format!(
                "GitHub preflight failed: identity mismatch — Composio github connection is \
                 `{composio_username}` but `git config user.name` is `{git_username}`. \
                 Either reconnect Composio under the right GitHub account or update \
                 `git config --global user.name` to match."
            ),
            GithubGateError::ComposioIdentityUnresolved => {
                "GitHub preflight failed: Composio GitHub connection is present but the \
                 connected username could not be resolved. Try reconnecting via \
                 `composio_authorize github`, then re-run."
                    .to_string()
            }
        };
        match log_path {
            Some(p) if !p.is_empty() => format!("{body} (gate log: {p})"),
            _ => body,
        }
    }

    /// Short tag suitable for the run-log header — keeps each failure
    /// reason grep-friendly.
    pub fn tag(&self) -> &'static str {
        match self {
            GithubGateError::ComposioGithubMissing => "composio_github_missing",
            GithubGateError::GitBinaryMissing(_) => "git_binary_missing",
            GithubGateError::GitUserNameMissing => "git_user_name_missing",
            GithubGateError::GitUserEmailMissing => "git_user_email_missing",
            GithubGateError::IdentityMismatch { .. } => "identity_mismatch",
            GithubGateError::ComposioIdentityUnresolved => "composio_identity_unresolved",
        }
    }
}

/// Side-effect probes the preflight needs. Production wires these to
/// real Composio + `git` calls (see [`LivePreflightProbes`]); tests
/// substitute a deterministic stub.
#[async_trait]
pub trait PreflightProbes: Send + Sync {
    /// True iff there is currently an active Composio connection for
    /// the given toolkit (per `fetch_connected_integrations`).
    async fn composio_toolkit_active(&self, toolkit: &str) -> bool;

    /// The connected account's identity (e.g. GitHub username) for the
    /// given toolkit. `None` ⇒ couldn't resolve (provider not
    /// registered, profile lookup failed, empty username).
    async fn composio_identity(&self, toolkit: &str) -> Option<String>;

    /// `git --version` outcome. `Ok(())` when git is on PATH and exits
    /// zero; `Err(message)` otherwise.
    async fn git_version(&self) -> Result<(), String>;

    /// `git config --global user.name` trimmed value, or empty when
    /// unset / blank.
    async fn git_user_name(&self) -> String;

    /// `git config --global user.email` trimmed value, or empty when
    /// unset / blank.
    async fn git_user_email(&self) -> String;
}

/// Production probes — hits real Composio (via
/// `composio::connection_identity` /
/// `composio::fetch_connected_integrations`) and the local `git`
/// binary via `tokio::process::Command`.
pub struct LivePreflightProbes<'a> {
    pub config: &'a Config,
}

impl<'a> LivePreflightProbes<'a> {
    pub fn new(config: &'a Config) -> Self {
        Self { config }
    }
}

#[async_trait]
impl<'a> PreflightProbes for LivePreflightProbes<'a> {
    async fn composio_toolkit_active(&self, toolkit: &str) -> bool {
        let connections = composio::fetch_connected_integrations(self.config).await;
        connections
            .iter()
            .any(|c| c.toolkit.eq_ignore_ascii_case(toolkit))
    }

    async fn composio_identity(&self, toolkit: &str) -> Option<String> {
        composio::connection_identity(self.config, toolkit).await
    }

    async fn git_version(&self) -> Result<(), String> {
        let fut = tokio::process::Command::new("git")
            .arg("--version")
            .output();
        match tokio::time::timeout(GIT_PROBE_TIMEOUT, fut).await {
            Ok(Ok(output)) if output.status.success() => Ok(()),
            Ok(Ok(output)) => Err(format!(
                "`git --version` exited {}",
                output.status.code().unwrap_or(-1)
            )),
            Ok(Err(e)) => Err(format!("`git --version` failed to spawn: {e}")),
            Err(_) => Err(format!(
                "`git --version` timed out after {}s",
                GIT_PROBE_TIMEOUT.as_secs()
            )),
        }
    }

    async fn git_user_name(&self) -> String {
        git_config_value("user.name").await
    }

    async fn git_user_email(&self) -> String {
        git_config_value("user.email").await
    }
}

async fn git_config_value(key: &str) -> String {
    let fut = tokio::process::Command::new("git")
        .args(["config", "--global", key])
        .output();
    match tokio::time::timeout(GIT_PROBE_TIMEOUT, fut).await {
        Ok(Ok(output)) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        // Non-zero exit, spawn failure, or timeout all collapse to the
        // existing empty-string fallback (treated as "unset").
        Ok(Ok(_)) | Ok(Err(_)) | Err(_) => String::new(),
    }
}

/// Run the GitHub preflight gate against the supplied [`PreflightProbes`].
///
/// Returns `Ok(())` when the gate accepts the run, or `Err(first
/// failure)` when any check fails. Checks run in the documented order
/// (Composio connection → git binary → git user.name → git user.email
/// → identity match) so the user sees the most foundational failure
/// first; we do not try to enumerate every problem at once.
///
/// The gate is a no-op (immediate `Ok(())`) when the `[github]` block
/// is absent (`cfg: None`) or `required = false`.
pub async fn run_github_preflight<P: PreflightProbes>(
    cfg: Option<&WorkflowGithubConfig>,
    probes: &P,
) -> Result<(), GithubGateError> {
    let Some(cfg) = cfg else {
        tracing::debug!("[workflows:preflight] github gate skipped: no [github] block");
        return Ok(());
    };
    if !cfg.required {
        tracing::debug!("[workflows:preflight] github gate skipped: required = false");
        return Ok(());
    }

    // (1) Composio GitHub integration must be connected.
    if !probes.composio_toolkit_active("github").await {
        tracing::warn!("[workflows:preflight] github gate fail: composio_github_missing");
        return Err(GithubGateError::ComposioGithubMissing);
    }

    // (2) git binary present.
    if let Err(e) = probes.git_version().await {
        tracing::warn!(error = %e, "[workflows:preflight] github gate fail: git_binary_missing");
        return Err(GithubGateError::GitBinaryMissing(e));
    }

    // (3a) git user.name set.
    let git_name = probes.git_user_name().await;
    if git_name.is_empty() {
        tracing::warn!("[workflows:preflight] github gate fail: git_user_name_missing");
        return Err(GithubGateError::GitUserNameMissing);
    }
    // (3b) git user.email set.
    let git_email = probes.git_user_email().await;
    if git_email.is_empty() {
        tracing::warn!("[workflows:preflight] github gate fail: git_user_email_missing");
        return Err(GithubGateError::GitUserEmailMissing);
    }

    // (4) Identity match, only when Strict.
    match cfg.identity_match {
        IdentityMatch::None => {
            tracing::debug!(
                "[workflows:preflight] github gate pass (identity_match=none, reachability only)"
            );
            Ok(())
        }
        IdentityMatch::Any => {
            // "any" still requires the Composio side to surface an
            // identity — confirms the connection is genuinely usable.
            match probes.composio_identity("github").await {
                Some(_) => {
                    tracing::debug!("[workflows:preflight] github gate pass (identity_match=any)");
                    Ok(())
                }
                None => {
                    tracing::warn!(
                        "[workflows:preflight] github gate fail: composio_identity_unresolved (identity_match=any)"
                    );
                    Err(GithubGateError::ComposioIdentityUnresolved)
                }
            }
        }
        IdentityMatch::Strict => {
            let composio_name = match probes.composio_identity("github").await {
                Some(n) => n,
                None => {
                    tracing::warn!(
                        "[workflows:preflight] github gate fail: composio_identity_unresolved (identity_match=strict)"
                    );
                    return Err(GithubGateError::ComposioIdentityUnresolved);
                }
            };
            if composio_name.trim().eq_ignore_ascii_case(git_name.trim()) {
                tracing::debug!(
                    composio = %composio_name,
                    git = %git_name,
                    "[workflows:preflight] github gate pass (identity_match=strict)"
                );
                Ok(())
            } else {
                tracing::warn!(
                    composio = %composio_name,
                    git = %git_name,
                    "[workflows:preflight] github gate fail: identity_mismatch"
                );
                Err(GithubGateError::IdentityMismatch {
                    composio_username: composio_name,
                    git_username: git_name,
                })
            }
        }
    }
}

#[cfg(test)]
#[path = "preflight_tests.rs"]
mod tests;
