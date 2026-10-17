//! Claims the persistent Chrome profile folder for a browser about to start.
//!
//! Which profile to use (persistent or fresh) and its folder are host policy,
//! read from Browser Control settings and sent with each claim. How Chrome
//! locks a folder, the wait for a closing browser and the claim bookkeeping
//! belong to TinyComputer (`BrowserClaimProfile` / `BrowserReleaseProfile`);
//! this adapter only asks and reports which outcome the module reached.

use serde_json::Value;
use tinycomputer_bus::browser::{
    names::methods, ClaimProfileRequest, ClaimProfileResponse, ProfileClaim, ReleaseProfileRequest,
};
use tinycomputer_bus::DesktopResponse;

use crate::config::Config;

/// How long a task waits, in milliseconds, for the Chrome holding its
/// persistent profile to let it go before it runs in a fresh profile.
pub(crate) const TASK_WAIT_MS: u64 = 5_000;

/// How long a browser session waits for the same: shorter than a task's, as
/// a person waits on it.
pub(crate) const SESSION_WAIT_MS: u64 = 1_000;

/// What a claim came to, for the caller to apply to its browser launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Claim {
    /// The folder is claimed: start the browser on it and release the claim
    /// if the browser does not start.
    Claimed(String),
    /// The folder is held by another browser (or the module could not be
    /// asked): start with a fresh profile.
    Fresh,
}

/// The request for `path`. The host runs a fresh profile by leaving the path
/// out, so the module is never asked to make a folder it would leak.
pub(crate) fn claim_request(path: &str, wait_ms: u64) -> ClaimProfileRequest {
    ClaimProfileRequest {
        path: path.to_owned(),
        wait_ms,
        fall_back_to_fresh: false,
    }
}

/// Reads the module's answer to a claim of `requested`.
pub(crate) fn settle(requested: &str, response: &ClaimProfileResponse) -> Claim {
    match (response.outcome, response.path.as_deref()) {
        (ProfileClaim::Claimed, path) => Claim::Claimed(path.unwrap_or(requested).to_owned()),
        (ProfileClaim::Busy | ProfileClaim::FellBackToFresh, _) => Claim::Fresh,
    }
}

/// Claims `path`, waiting up to `wait_ms` for a Chrome that is closing on it.
/// A module that cannot be asked leaves the folder unclaimed and the browser
/// on a fresh profile, as a held folder does.
pub(crate) async fn claim(config: &Config, path: &str, wait_ms: u64) -> Claim {
    let reply = call(
        config,
        methods::CLAIM_PROFILE,
        &claim_request(path, wait_ms),
    )
    .await;
    let Some(data) = reply else {
        tracing::warn!("[profile-claim] the module could not claim the profile; using a fresh one");
        return Claim::Fresh;
    };
    match serde_json::from_value::<ClaimProfileResponse>(data) {
        Ok(response) => {
            let claim = settle(path, &response);
            tracing::debug!(outcome = ?response.outcome, wait_ms, "[profile-claim] claimed");
            claim
        }
        Err(error) => {
            tracing::warn!(%error, "[profile-claim] unreadable claim reply; using a fresh profile");
            Claim::Fresh
        }
    }
}

/// Gives up a claim: the browser did not start, or holds the folder itself.
pub(crate) async fn release(config: &Config, path: &str) {
    let request = ReleaseProfileRequest {
        path: path.to_owned(),
    };
    let _ = call(config, methods::RELEASE_PROFILE, &request).await;
}

async fn call<A: serde::Serialize + Sync>(
    config: &Config,
    member: &str,
    request: &A,
) -> Option<Value> {
    let proxy = super::desktop::proxy(config).await.ok()?;
    let response: DesktopResponse = proxy.call(member, (request,)).await.ok()?;
    if !response.ok {
        tracing::debug!(member, "[profile-claim] the module refused the call");
        return None;
    }
    Some(response.data.unwrap_or(Value::Null))
}

#[cfg(test)]
#[path = "profile_claim_tests.rs"]
mod tests;
