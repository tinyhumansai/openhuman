//! RPC handler implementations for keyring consent.

use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;

use super::policy;
use super::types::{ConsentPreference, KeyringStatus};

const LOG_PREFIX: &str = "[keyring_consent]";

pub async fn keyring_status() -> Result<Outcome<KeyringStatus>, String> {
    let status = policy::current_status();
    log::debug!(
        "{LOG_PREFIX} keyring_status available={} mode={} backend={}",
        status.available,
        status.active_mode,
        status.backend_name,
    );
    Ok(Outcome::single_log(status, "keyring status fetched"))
}

pub async fn keyring_consent_decide(
    ctx: &DispatchCtx,
    mode: String,
) -> Result<Outcome<ConsentPreference>, String> {
    if mode != "local_encrypted" && mode != "declined" {
        return Err(format!(
            "invalid mode '{mode}': expected 'local_encrypted' or 'declined'"
        ));
    }
    log::info!("{LOG_PREFIX} keyring_consent_decide mode={mode}");

    // Build the preference value without touching the in-memory cache yet.
    let pref = policy::build_consent_preference(&mode);

    // Persist to disk first. If this fails we return an error without
    // updating the cache, so cache and disk stay consistent.
    persist_consent(ctx, &pref).await?;

    // Only update the in-memory cache after a successful persist.
    policy::apply_consent(&pref);

    Ok(Outcome::single_log(
        pref,
        format!("keyring consent recorded: {mode}"),
    ))
}

pub async fn keyring_retry_probe() -> Result<Outcome<KeyringStatus>, String> {
    log::info!("{LOG_PREFIX} keyring_retry_probe");
    let status = policy::retry_probe();
    Ok(Outcome::single_log(
        status,
        "keyring probe retried".to_string(),
    ))
}

async fn persist_consent(ctx: &DispatchCtx, pref: &ConsentPreference) -> Result<(), String> {
    let patch = crate::desktop::app_state::StoredAppStatePatch {
        keyring_consent: Some(Some(pref.clone())),
        ..Default::default()
    };
    crate::desktop::app_state::update_local_state(ctx, patch).await?;
    log::debug!("{LOG_PREFIX} consent persisted to app state");
    Ok(())
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
