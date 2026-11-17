//! Direct mode (BYO API key) ops.

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;
use crate::credentials::{AuthService, DEFAULT_AUTH_PROFILE_NAME};

use super::super::{
    client::{create_direct_client_for_api_key, direct_list_connections},
    direct_auth,
};
use super::error_utils::OpResult;

/// Provider slot for the user-provided Composio API key in direct mode.
pub const COMPOSIO_DIRECT_PROVIDER: &str = "composio-direct";

/// Store the user-provided Composio key in the encrypted credential store.
pub async fn store_composio_api_key(
    config: &Config,
    api_key: &str,
) -> Result<Outcome<serde_json::Value>, String> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Err("composio api_key must not be empty".to_string());
    }
    tracing::debug!(
        len = trimmed.len(),
        "[composio-direct] storing api key (redacted)"
    );
    AuthService::from_config(config)
        .store_provider_token(
            COMPOSIO_DIRECT_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            trimmed,
            std::collections::HashMap::new(),
            true,
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(Outcome::single_log(
        serde_json::json!({ "stored": true, "provider": COMPOSIO_DIRECT_PROVIDER }),
        "composio direct api key stored",
    ))
}

/// Read the stored direct-mode Composio API key, if present.
pub async fn get_composio_api_key(config: &Config) -> Result<Option<String>, String> {
    let key = AuthService::from_config(config)
        .get_provider_bearer_token(COMPOSIO_DIRECT_PROVIDER, None)
        .await
        .map_err(|error| error.to_string())?;
    Ok(key
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty()))
}

/// Remove the stored direct-mode Composio API key.
pub async fn clear_composio_api_key(config: &Config) -> Result<Outcome<serde_json::Value>, String> {
    tracing::debug!("[composio-direct] clearing stored api key");
    let removed = AuthService::from_config(config)
        .remove_profile(COMPOSIO_DIRECT_PROVIDER, DEFAULT_AUTH_PROFILE_NAME)
        .await
        .map_err(|error| error.to_string())?;
    Ok(Outcome::single_log(
        serde_json::json!({ "removed": removed }),
        "composio direct api key cleared",
    ))
}

/// RPC wrapper accepting a plain string and storing only its trimmed value.
pub async fn rpc_store_composio_api_key(
    config: &Config,
    api_key: &str,
) -> Result<Outcome<serde_json::Value>, String> {
    store_composio_api_key(config, api_key).await
}

async fn validate_direct_api_key_before_store(config: &Config, api_key: &str) -> OpResult<()> {
    let key_id = direct_auth::fingerprint_api_key(api_key);
    direct_auth::reset_direct_auth_failure(key_id);

    let direct = create_direct_client_for_api_key(api_key)
        .map_err(|e| format!("[composio-direct] validate_api_key: {e}"))?;

    match direct_list_connections(config, &direct).await {
        Ok(_) => {
            direct_auth::record_direct_auth_success(key_id);
            tracing::debug!("[composio-direct] validate_api_key: probe succeeded");
            Ok(())
        }
        Err(error) => {
            let rendered = format!("{error:#}");
            if direct_auth::is_invalid_api_key_error(&rendered) {
                tracing::warn!(
                    "[composio-direct] validate_api_key: rejected invalid API key before storage"
                );
                Err(direct_auth::COMPOSIO_INVALID_API_KEY_USER_MESSAGE.into())
            } else {
                tracing::warn!(
                    error = %rendered,
                    "[composio-direct] validate_api_key: probe failed for a non-auth reason; \
                     storing key so the user can retry when Composio is reachable"
                );
                Ok(())
            }
        }
    }
}

/// Read the current Composio routing mode and whether a direct-mode API
/// key is stored. **The key itself is never returned** — only a boolean
/// flag so the UI can show a "Connected" / "Not set" status.
pub async fn composio_get_mode(config: &Config) -> OpResult<Outcome<serde_json::Value>> {
    let mode = config.composio.mode.trim().to_string();
    let key_present = get_composio_api_key(config)
        .await
        .map_err(|e| format!("[composio-direct] get_composio_api_key failed: {e}"))?
        .is_some();
    tracing::debug!(
        mode = %mode,
        key_present = key_present,
        "[composio-direct] get_mode"
    );
    let payload = serde_json::json!({
        "mode": mode,
        "api_key_set": key_present,
    });
    Ok(Outcome::new(
        payload,
        vec![format!(
            "composio: mode={mode}, api_key={}",
            if key_present { "set" } else { "unset" }
        )],
    ))
}

/// Persist a user-provided Composio API key for direct mode and
/// (optionally) flip `config.composio.mode` over to `"direct"`.
///
/// **Logging redacts the key** — only its length and presence are recorded.
pub async fn composio_set_api_key(
    ctx: &DispatchCtx,
    config: &Config,
    api_key: &str,
    activate_direct: bool,
) -> OpResult<Outcome<serde_json::Value>> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Err("composio.set_api_key: api_key must not be empty".to_string());
    }
    tracing::debug!(
        key_len = trimmed.len(),
        activate_direct,
        "[composio-direct] set_api_key (redacted)"
    );
    validate_direct_api_key_before_store(config, trimmed).await?;

    store_composio_api_key(config, trimmed)
        .await
        .map_err(|e| format!("[composio-direct] store_composio_api_key failed: {e}"))?;

    let mode_log = if activate_direct {
        let mut cfg_mut = ctx
            .load_config()
            .await
            .map_err(|e| format!("[composio-direct] reload config failed: {e}"))?;
        cfg_mut.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.into();
        cfg_mut.save().await.map_err(|error| {
            crate::config::ops::config_save_error_with_context(
                error,
                "[composio-direct] save config failed",
                false,
            )
        })?;
        "mode=direct"
    } else {
        "mode unchanged"
    };

    let effective_mode: String = if activate_direct {
        "direct".to_string()
    } else {
        config.composio.mode.clone()
    };

    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::ComposioConfigChanged {
        mode: effective_mode.clone(),
        api_key_set: true,
        tenant: ctx.event_tenant(),
    });
    tracing::debug!(
        mode = %effective_mode,
        "[composio-cache] published ComposioConfigChanged after set_api_key"
    );

    Ok(Outcome::new(
        serde_json::json!({
            "stored": true,
            "mode": effective_mode,
        }),
        vec![format!("composio: api key stored ({mode_log})")],
    ))
}

/// Clear the stored direct-mode API key and reset
/// `config.composio.mode` back to `"backend"`.
pub async fn composio_clear_api_key(
    ctx: &DispatchCtx,
    config: &Config,
) -> OpResult<Outcome<serde_json::Value>> {
    tracing::debug!("[composio-direct] clear_api_key");
    clear_composio_api_key(config)
        .await
        .map_err(|e| format!("[composio-direct] clear_composio_api_key failed: {e}"))?;

    let mut cfg_mut = ctx
        .load_config()
        .await
        .map_err(|e| format!("[composio-direct] reload config failed: {e}"))?;
    cfg_mut.composio.mode = crate::config::schema::COMPOSIO_MODE_BACKEND.into();
    cfg_mut.save().await.map_err(|error| {
        crate::config::ops::config_save_error_with_context(
            error,
            "[composio-direct] save config failed",
            false,
        )
    })?;
    direct_auth::reset_all_direct_auth_failures();

    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::ComposioConfigChanged {
        mode: "backend".to_string(),
        api_key_set: false,
        tenant: ctx.event_tenant(),
    });
    tracing::debug!("[composio-cache] published ComposioConfigChanged after clear_api_key");

    Ok(Outcome::new(
        serde_json::json!({ "cleared": true, "mode": "backend" }),
        vec!["composio: api key cleared, mode reset to backend".into()],
    ))
}
