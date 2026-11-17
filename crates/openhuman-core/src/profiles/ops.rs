//! Operator-plane operations on profiles.
//!
//! Gateway user ids are taken here and turned into profile ids at once. The
//! user id itself is never logged, stored or returned; under raw `profile_ids`
//! mode a qualifying id *is* the profile id, so it is stored and returned as
//! one (use `"hashed"` where that must not happen). Log lines and `Outcome`
//! messages stay content-free either way.

use super::credentials::{self, UserCredentialKind};
use super::host::{self, ProfileHost};
use super::types::{
    CredentialResult, DeprovisionResult, ProfileId, ProfileSummary, ProvisionResult, ReleaseResult,
};
use crate::core::Outcome;

fn require_host() -> Result<std::sync::Arc<ProfileHost>, String> {
    host::host().ok_or_else(|| "profiles exist only in SaaS mode".to_string())
}

/// Create the profile for gateway user `user_id`, if it does not exist yet.
pub async fn provision(user_id: &str) -> Result<Outcome<ProvisionResult>, String> {
    provision_on(&*require_host()?, user_id).await
}

pub(crate) async fn provision_on(
    host: &ProfileHost,
    user_id: &str,
) -> Result<Outcome<ProvisionResult>, String> {
    let profile_id = ProfileId::for_user(user_id, host.saas().profile_ids)?;
    if names_operator_state(&profile_id, &host.saas().operator_dir()) {
        log::warn!(
            "[profiles] refusing to provision a profile named like the operator's state dir"
        );
        return Err(
            "profile id is reserved on this deployment: it is the name of the \
                    operator's state directory"
                .to_string(),
        );
    }
    let created = host.provision(&profile_id).await?;
    let log = if created {
        "profile provisioned"
    } else {
        "profile already provisioned"
    };
    Ok(Outcome::single_log(
        ProvisionResult {
            profile_id,
            created,
        },
        log,
    ))
}

/// Whether `id` is the file name of the operator's state directory.
///
/// Credential secrets are namespaced in the process keyring by the name of
/// the directory their store sits in: `users/<id>` for a profile, the
/// operator directory for the operator. A profile named like the operator
/// directory (`operator` is reserved for the default; a configured
/// `operator_dir` can be any name) would read and overwrite the operator's
/// own secrets.
fn names_operator_state(id: &ProfileId, operator_dir: &std::path::Path) -> bool {
    operator_dir.file_name().and_then(|name| name.to_str()) == Some(id.as_str())
}

/// Close profile `profile_id` and archive its state.
pub async fn deprovision(profile_id: &str) -> Result<Outcome<DeprovisionResult>, String> {
    deprovision_on(&*require_host()?, profile_id).await
}

pub(crate) async fn deprovision_on(
    host: &ProfileHost,
    profile_id: &str,
) -> Result<Outcome<DeprovisionResult>, String> {
    let profile_id = ProfileId::parse(profile_id)?;
    let removed = host.deprovision(&profile_id).await?;
    let log = if removed {
        "profile archived"
    } else {
        "profile was not provisioned"
    };
    Ok(Outcome::single_log(
        DeprovisionResult {
            profile_id,
            removed,
        },
        log,
    ))
}

/// Every provisioned profile.
pub async fn list() -> Result<Outcome<Vec<ProfileSummary>>, String> {
    let profiles = require_host()?.list().await?;
    let log = format!("{} profile(s)", profiles.len());
    Ok(Outcome::single_log(profiles, log))
}

/// One profile, or an error when it is not provisioned.
pub async fn status(profile_id: &str) -> Result<Outcome<ProfileSummary>, String> {
    status_on(&*require_host()?, profile_id).await
}

pub(crate) async fn status_on(
    host: &ProfileHost,
    profile_id: &str,
) -> Result<Outcome<ProfileSummary>, String> {
    let profile_id = ProfileId::parse(profile_id)?;
    let summary = host
        .summary(&profile_id)
        .await?
        .ok_or_else(|| "profile is not provisioned".to_string())?;
    Ok(Outcome::single_log(summary, "profile status read"))
}

/// Install the backend credential the gateway holds for profile `profile_id`.
pub async fn set_credential(
    profile_id: &str,
    kind: UserCredentialKind,
    token: &str,
    expires_at: Option<&str>,
) -> Result<Outcome<CredentialResult>, String> {
    set_credential_on(&*require_host()?, profile_id, kind, token, expires_at).await
}

pub(crate) async fn set_credential_on(
    host: &ProfileHost,
    profile_id: &str,
    kind: UserCredentialKind,
    token: &str,
    expires_at: Option<&str>,
) -> Result<Outcome<CredentialResult>, String> {
    let profile_id = ProfileId::parse(profile_id)?;
    let token = token.to_string();
    let expires_at = expires_at.map(str::to_string);
    // From the layout, not `open`: installing or revoking a credential must
    // work even when every profile slot is busy. Under the profile's
    // lifecycle lock, so it cannot land in a profile being archived, and
    // under its own scope, so a storage-backed secret store files the
    // credential under that profile.
    host.with_records(&profile_id, |config| {
        Box::pin(
            async move { credentials::store(config, kind, &token, expires_at.as_deref()).await },
        )
    })
    .await?;
    log::info!("[profiles] credential installed kind={kind:?}");
    Ok(Outcome::single_log(
        CredentialResult {
            profile_id: profile_id.clone(),
            has_credential: true,
        },
        "credential installed",
    ))
}

/// Remove every credential profile `profile_id` holds.
pub async fn clear_credential(profile_id: &str) -> Result<Outcome<CredentialResult>, String> {
    clear_credential_on(&*require_host()?, profile_id).await
}

pub(crate) async fn clear_credential_on(
    host: &ProfileHost,
    profile_id: &str,
) -> Result<Outcome<CredentialResult>, String> {
    let profile_id = ProfileId::parse(profile_id)?;
    let removed = host
        .with_records(&profile_id, |config| Box::pin(credentials::clear(config)))
        .await?;
    log::info!("[profiles] credential cleared removed={removed}");
    let log = if removed {
        "credential cleared"
    } else {
        "profile held no credential"
    };
    Ok(Outcome::single_log(
        CredentialResult {
            profile_id,
            has_credential: false,
        },
        log,
    ))
}

/// Close profile `profile_id` on this node and release its lease, so another
/// node can host it at once.
pub async fn release(profile_id: &str) -> Result<Outcome<ReleaseResult>, String> {
    release_on(&*require_host()?, profile_id).await
}

pub(crate) async fn release_on(
    host: &ProfileHost,
    profile_id: &str,
) -> Result<Outcome<ReleaseResult>, String> {
    let profile_id = ProfileId::parse(profile_id)?;
    let released = host.release(&profile_id).await?;
    let log = if released {
        "profile released"
    } else {
        "profile was not open on this node"
    };
    Ok(Outcome::single_log(
        ReleaseResult {
            profile_id,
            released,
        },
        log,
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
