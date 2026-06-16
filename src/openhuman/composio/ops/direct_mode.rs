//! Direct mode (BYO API key) ops.

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::error_utils::OpResult;

/// Read the current Composio routing mode and whether a direct-mode API
/// key is stored. **The key itself is never returned** — only a boolean
/// flag so the UI can show a "Connected" / "Not set" status.
pub async fn composio_get_mode(config: &Config) -> OpResult<RpcOutcome<serde_json::Value>> {
    let mode = config.composio.mode.trim().to_string();
    let key_present = crate::openhuman::credentials::get_composio_api_key(config)
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
    Ok(RpcOutcome::new(
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
    config: &Config,
    api_key: &str,
    activate_direct: bool,
) -> OpResult<RpcOutcome<serde_json::Value>> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Err("composio.set_api_key: api_key must not be empty".to_string());
    }
    tracing::debug!(
        key_len = trimmed.len(),
        activate_direct,
        "[composio-direct] set_api_key (redacted)"
    );

    crate::openhuman::credentials::store_composio_api_key(config, trimmed)
        .await
        .map_err(|e| format!("[composio-direct] store_composio_api_key failed: {e}"))?;

    let mode_log = if activate_direct {
        let mut cfg_mut = crate::openhuman::config::rpc::load_config_with_timeout()
            .await
            .map_err(|e| format!("[composio-direct] reload config failed: {e}"))?;
        cfg_mut.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.into();
        cfg_mut
            .save()
            .await
            .map_err(|e| format!("[composio-direct] save config failed: {e}"))?;
        "mode=direct"
    } else {
        "mode unchanged"
    };

    let effective_mode: String = if activate_direct {
        "direct".to_string()
    } else {
        config.composio.mode.clone()
    };

    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::ComposioConfigChanged {
            mode: effective_mode.clone(),
            api_key_set: true,
        },
    );
    tracing::debug!(
        mode = %effective_mode,
        "[composio-cache] published ComposioConfigChanged after set_api_key"
    );

    Ok(RpcOutcome::new(
        serde_json::json!({
            "stored": true,
            "mode": effective_mode,
        }),
        vec![format!("composio: api key stored ({mode_log})")],
    ))
}

/// Clear the stored direct-mode API key and reset
/// `config.composio.mode` back to `"backend"`.
pub async fn composio_clear_api_key(config: &Config) -> OpResult<RpcOutcome<serde_json::Value>> {
    tracing::debug!("[composio-direct] clear_api_key");
    crate::openhuman::credentials::clear_composio_api_key(config)
        .await
        .map_err(|e| format!("[composio-direct] clear_composio_api_key failed: {e}"))?;

    let mut cfg_mut = crate::openhuman::config::rpc::load_config_with_timeout()
        .await
        .map_err(|e| format!("[composio-direct] reload config failed: {e}"))?;
    cfg_mut.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_BACKEND.into();
    cfg_mut
        .save()
        .await
        .map_err(|e| format!("[composio-direct] save config failed: {e}"))?;

    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::ComposioConfigChanged {
            mode: "backend".to_string(),
            api_key_set: false,
        },
    );
    tracing::debug!("[composio-cache] published ComposioConfigChanged after clear_api_key");

    Ok(RpcOutcome::new(
        serde_json::json!({ "cleared": true, "mode": "backend" }),
        vec!["composio: api key cleared, mode reset to backend".into()],
    ))
}
