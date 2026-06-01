//! RPC handler implementations for keyring consent.

use crate::rpc::RpcOutcome;

use super::policy;
use super::types::{ConsentPreference, KeyringStatus};

const LOG_PREFIX: &str = "[keyring_consent]";

pub async fn keyring_status() -> Result<RpcOutcome<KeyringStatus>, String> {
    let status = policy::current_status();
    log::debug!(
        "{LOG_PREFIX} keyring_status available={} mode={} backend={}",
        status.available,
        status.active_mode,
        status.backend_name,
    );
    Ok(RpcOutcome::single_log(status, "keyring status fetched"))
}

pub async fn keyring_consent_decide(mode: String) -> Result<RpcOutcome<ConsentPreference>, String> {
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
    persist_consent(&pref).await?;

    // Only update the in-memory cache after a successful persist.
    policy::apply_consent(&pref);

    Ok(RpcOutcome::single_log(
        pref,
        format!("keyring consent recorded: {mode}"),
    ))
}

pub async fn keyring_retry_probe() -> Result<RpcOutcome<KeyringStatus>, String> {
    log::info!("{LOG_PREFIX} keyring_retry_probe");
    let status = policy::retry_probe();
    Ok(RpcOutcome::single_log(
        status,
        "keyring probe retried".to_string(),
    ))
}

async fn persist_consent(pref: &ConsentPreference) -> Result<(), String> {
    let patch = crate::openhuman::app_state::StoredAppStatePatch {
        keyring_consent: Some(Some(pref.clone())),
        ..Default::default()
    };
    crate::openhuman::app_state::update_local_state(patch).await?;
    log::debug!("{LOG_PREFIX} consent persisted to app state");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn keyring_status_returns_ok() {
        let result = keyring_status().await;
        assert!(result.is_ok());
        let outcome = result.unwrap();
        assert!(!outcome.value.backend_name.is_empty());
    }

    #[tokio::test]
    async fn keyring_consent_decide_rejects_invalid_mode() {
        let result = keyring_consent_decide("invalid".to_string()).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid mode"));
    }

    #[tokio::test]
    async fn keyring_retry_probe_returns_ok() {
        let result = keyring_retry_probe().await;
        assert!(result.is_ok());
    }
}
