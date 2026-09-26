//! Config snapshots serialized for the desktop UI.

use serde_json::json;

use super::load::load_config_with_timeout;
use crate::config::Config;
use crate::rpc::RpcOutcome;

/// Serializes the current configuration into a JSON snapshot for the UI.
pub fn snapshot_config_json(config: &Config) -> Result<serde_json::Value, String> {
    let mut value = serde_json::to_value(config).map_err(|e| e.to_string())?;
    // The full snapshot is sent over RPC. Keep search settings visible while
    // removing credentials, including the legacy Seltz key.
    for provider in ["parallel", "brave", "querit", "exa", "tavily", "gemini"] {
        value["search"][provider]["api_key"] = serde_json::Value::Null;
    }
    value["seltz"]["api_key"] = serde_json::Value::Null;
    #[cfg(feature = "modules")]
    let browser_billing_route = match crate::modules::browser_task::billing_route(config) {
        crate::modules::browser_task::BillingRoute::DirectOpenRouter => "direct_openrouter",
        crate::modules::browser_task::BillingRoute::Hosted => "hosted",
    };
    #[cfg(not(feature = "modules"))]
    let browser_billing_route = "unavailable";
    Ok(json!({
        "config": value,
        "browser_billing_route": browser_billing_route,
        "workspace_dir": config.workspace_dir.display().to_string(),
        "config_path": config.config_path.display().to_string(),
    }))
}

/// Serializes the client-facing AI config slice consumed by the settings UI.
pub fn client_config_json(config: &Config) -> serde_json::Value {
    let app_version =
        std::env::var("OPENHUMAN_APP_VERSION").unwrap_or_else(|_| "unknown".to_string());
    let api_key_set = config
        .api_key
        .as_deref()
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false);
    let model_routes: Vec<serde_json::Value> = config
        .model_routes
        .iter()
        .map(|r| serde_json::json!({ "hint": r.hint, "model": r.model }))
        .collect();
    let cloud_providers: Vec<serde_json::Value> = config
        .cloud_providers
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.id,
                "slug": c.slug,
                "label": c.label,
                "endpoint": c.endpoint,
                "auth_style": c.auth_style.as_str(),
            })
        })
        .collect();
    let model_registry: Vec<serde_json::Value> = config
        .model_registry
        .iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "provider": m.provider,
                "cost_per_1m_input": m.cost_per_1m_input,
                "cost_per_1m_cached_input": m.cost_per_1m_cached_input,
                "cost_per_1m_output": m.cost_per_1m_output,
                "context_window": m.context_window,
                "vision": m.vision,
            })
        })
        .collect();

    serde_json::json!({
        "api_url": config.api_url,
        "inference_url": config.inference_url,
        "default_model": config.default_model,
        "app_version": app_version,
        "api_key_set": api_key_set,
        "model_routes": model_routes,
        "cloud_providers": cloud_providers,
        "model_registry": model_registry,
        "primary_cloud": config.primary_cloud,
        // #3767: authoritative, core-side decision telling the UI whether the
        // managed-credits gate should be bypassed, per chat-mode tier. The chat
        // header's "Quick" mode runs on the `chat` tier and "Reasoning" mode on
        // the `reasoning` tier, so each is reported separately and the UI checks
        // the tier the user actually selected. True for a tier when it runs on a
        // non-managed provider the user funds themselves (BYO key / local /
        // claude-code) with usable creds. Managed tiers that run anyway surface
        // credit errors per-call.
        "credits_bypass": {
            "chat": crate::inference::provider::factory::role_bypasses_managed_credits(
                "chat", config,
            ),
            "reasoning":
                crate::inference::provider::factory::role_bypasses_managed_credits(
                    "reasoning", config,
                ),
        },
        "chat_provider": config.chat_provider,
        "reasoning_provider": config.reasoning_provider,
        "agentic_provider": config.agentic_provider,
        "coding_provider": config.coding_provider,
        "vision_provider": config.vision_provider,
        "memory_provider": config.memory_provider,
        "embeddings_provider": config.embeddings_provider,
        "heartbeat_provider": config.heartbeat_provider,
        "learning_provider": config.learning_provider,
        "subconscious_provider": config.subconscious_provider,
        "voice_providers": config.voice_providers.iter().map(|v| {
            serde_json::json!({
                "id": v.id,
                "slug": v.slug,
                "label": v.label,
                "endpoint": v.endpoint,
                "auth_style": v.auth_style.as_str(),
                "capability": v.capability.as_str(),
                "stt_api_style": v.stt_api_style,
                "tts_api_style": v.tts_api_style,
                "default_stt_model": v.default_stt_model,
                "default_tts_voice": v.default_tts_voice,
            })
        }).collect::<Vec<_>>(),
        "stt_provider": config.stt_provider,
        "tts_provider": config.tts_provider,
    })
}

/// Loads config and returns the client-facing AI config slice.
pub async fn load_and_get_client_config_snapshot() -> Result<RpcOutcome<serde_json::Value>, String>
{
    let config = load_config_with_timeout().await?;
    let snapshot = client_config_json(&config);
    Ok(RpcOutcome::new(
        snapshot,
        vec!["client config read".to_string()],
    ))
}

/// Returns a full configuration snapshot for the UI.
pub async fn get_config_snapshot(config: &Config) -> Result<RpcOutcome<serde_json::Value>, String> {
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "config loaded from {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration from disk and returns a snapshot.
pub async fn load_and_get_config_snapshot() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    get_config_snapshot(&config).await
}

/// Reads dashboard settings exposed to the desktop UI.
pub async fn get_dashboard_settings() -> Result<RpcOutcome<serde_json::Value>, String> {
    let request_id = uuid::Uuid::new_v4().to_string();
    tracing::debug!(
        target: "openhuman_core::config",
        request_id = %request_id,
        method = "openhuman.config_get_dashboard_settings",
        "OPENHUMAN: get_dashboard_settings entry"
    );
    tracing::debug!(
        target: "openhuman_core::config",
        request_id = %request_id,
        method = "openhuman.config_get_dashboard_settings",
        "OPENHUMAN: get_dashboard_settings loading config"
    );

    let config = load_config_with_timeout().await.map_err(|error| {
        tracing::warn!(
            target: "openhuman_core::config",
            request_id = %request_id,
            method = "openhuman.config_get_dashboard_settings",
            error = %error,
            "OPENHUMAN: get_dashboard_settings config load failed"
        );
        error
    })?;

    tracing::debug!(
        target: "openhuman_core::config",
        request_id = %request_id,
        method = "openhuman.config_get_dashboard_settings",
        "OPENHUMAN: get_dashboard_settings serializing dashboard settings"
    );
    let result = serde_json::to_value(&config.dashboard).map_err(|error| {
        let message = error.to_string();
        tracing::warn!(
            target: "openhuman_core::config",
            request_id = %request_id,
            method = "openhuman.config_get_dashboard_settings",
            error = %message,
            "OPENHUMAN: get_dashboard_settings serialization failed"
        );
        message
    })?;

    tracing::debug!(
        target: "openhuman_core::config",
        request_id = %request_id,
        method = "openhuman.config_get_dashboard_settings",
        "OPENHUMAN: get_dashboard_settings exit"
    );
    Ok(RpcOutcome::new(
        result,
        vec!["dashboard settings read".to_string()],
    ))
}
