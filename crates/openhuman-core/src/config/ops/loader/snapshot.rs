//! Config snapshots serialized for the desktop UI.

use crate::core::runtime::DispatchCtx;
use serde_json::json;

use crate::config::schema::load::{secret_slots, SecretSlot};
use crate::config::Config;
use crate::core::Outcome;

/// Serializes the current configuration into a JSON snapshot for the UI.
pub async fn snapshot_config_json(config: &Config) -> Result<serde_json::Value, String> {
    // The full snapshot is sent over RPC and returned to the model by the
    // `config_snapshot` tool, so no credential leaves in it.
    let mut masked = config.clone();
    for (_, slot) in secret_slots(&mut masked) {
        match slot {
            SecretSlot::Opt(value) => *value = None,
            SecretSlot::Str(value) => value.clear(),
        }
    }
    mask_unsealed_credentials(&mut masked);
    let mut value = serde_json::to_value(&masked).map_err(|e| e.to_string())?;
    // Channel secrets that are not sealed on disk are not in that list.
    mask_secret_named_strings_outside_data_keyed_maps(&mut value);
    // A broker URL may carry credentials too; show it redacted.
    if let Some(url) = config.inbound.queue_url.as_deref() {
        value["inbound"]["queue_url"] =
            serde_json::Value::String(crate::config::schema::storage::redact_url(url));
    }
    // A storage URL may carry database credentials; show it redacted.
    if let Some(url) = config.storage.url.as_deref() {
        value["storage"]["url"] =
            serde_json::Value::String(crate::config::schema::storage::redact_url(url));
    }
    #[cfg(feature = "modules")]
    let browser_billing_route = crate::modules::desktop::billing_route(config).await;
    #[cfg(not(feature = "modules"))]
    let browser_billing_route = "unavailable";
    Ok(json!({
        "config": value,
        "browser_billing_route": browser_billing_route,
        "workspace_dir": config.workspace_dir.display().to_string(),
        "config_path": config.config_path.display().to_string(),
    }))
}

/// Credentials the config holds outside the sealed list: MCP server auth,
/// environment, arguments and endpoint, MCP client capabilities, and proxy
/// URLs, which may carry userinfo.
fn mask_unsealed_credentials(config: &mut Config) {
    use crate::config::schema::storage::redact_url;
    use crate::config::McpAuthConfig;
    for server in &mut config.mcp_client.servers {
        let server = &mut server.server;
        // Any variable can hold a key (`AWS_SECRET_ACCESS_KEY`), so every
        // value goes; the names stay.
        server.env.values_mut().for_each(String::clear);
        // An argument can be a key on its own (`--token`, `<key>`), with no
        // name to match, so every argument goes; the count stays.
        server.args.iter_mut().for_each(String::clear);
        server.endpoint = redact_url(&server.endpoint);
        match server.auth {
            McpAuthConfig::None => {}
            McpAuthConfig::BearerToken { ref mut token } => token.clear(),
            McpAuthConfig::Basic {
                ref mut password, ..
            } => password.clear(),
            McpAuthConfig::Header { ref mut value, .. }
            | McpAuthConfig::QueryParam { ref mut value, .. } => value.clear(),
            McpAuthConfig::Headers { ref mut headers } => {
                headers.iter_mut().for_each(|header| header.value.clear())
            }
            // A kind added upstream is dropped whole until it is listed here.
            _ => server.auth = McpAuthConfig::None,
        }
    }
    // Client-declared metadata with free-form keys (an `Authorization` entry
    // has no credential-shaped name), and nothing displays it.
    blank_strings(&mut config.mcp_client.client_identity.capabilities);
    let proxy = &mut config.proxy;
    for url in [
        &mut proxy.http_proxy,
        &mut proxy.https_proxy,
        &mut proxy.all_proxy,
    ]
    .into_iter()
    .flatten()
    {
        *url = redact_url(url);
    }
}

/// Maps keyed by data (model ids, channel names) rather than field names,
/// whose values hold no credential. MCP `env` is not here: its values are
/// cleared by type.
const DATA_KEYED_MAPS: &[&str] = &[
    "/agent/channel_agents",
    "/agent/channel_permissions",
    "/channel_voice/channels",
    "/channel_voice/threads",
    "/cloud_provider_ca_certs",
    "/cost/prices",
    "/inbound/sources",
    "/inbound_debounce/by_channel",
    "/memory/agents",
    "/memory/engines",
    "/reliability/model_fallbacks",
    "/runtime/reasoning_effort_by_model",
    "/search/providers",
    "/search/roles",
    "/teams",
];

/// [`mask_secret_named_strings`] over the whole config, except that the keys of
/// a [`DATA_KEYED_MAPS`] map are not read as field names (a model id ending in
/// `token` is not a credential); their values are still walked.
pub(super) fn mask_secret_named_strings_outside_data_keyed_maps(config: &mut serde_json::Value) {
    // Free-form agent metadata: like MCP capabilities, its keys say nothing,
    // so every string in it goes.
    if let Some(serde_json::Value::Array(entries)) = config.pointer_mut("/agent_registry/entries") {
        entries
            .iter_mut()
            .filter_map(|entry| entry.get_mut("metadata"))
            .for_each(blank_strings);
    }
    let held: Vec<_> = DATA_KEYED_MAPS
        .iter()
        .filter_map(|at| Some((*at, config.pointer_mut(at)?.take())))
        .collect();
    mask_secret_named_strings(config);
    for (at, mut map) in held {
        if let serde_json::Value::Object(entries) = &mut map {
            entries.values_mut().for_each(mask_secret_named_strings);
        }
        if let Some(slot) = config.pointer_mut(at) {
            *slot = map;
        }
    }
}

/// Blanks every string under a key that names a credential (`password`,
/// `*_secret`, `*_token`, `*_key` credentials), and redacts the userinfo and
/// query of every string under a key that names a URL (`url`, `*_url`, `*endpoint`,
/// `*_dsn`), at any depth, including inside a list or object under the key.
fn mask_secret_named_strings(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if names_a_secret(key) {
                    blank_strings(child);
                } else if names_a_url(key) {
                    redact_url_strings(child);
                } else {
                    mask_secret_named_strings(child);
                }
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(mask_secret_named_strings),
        _ => {}
    }
}

fn blank_strings(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => text.clear(),
        serde_json::Value::Array(items) => items.iter_mut().for_each(blank_strings),
        serde_json::Value::Object(map) => map.values_mut().for_each(blank_strings),
        _ => {}
    }
}

fn redact_url_strings(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            *text = crate::config::schema::storage::redact_url(text);
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(redact_url_strings),
        // An object under a URL key can still hold a credential-named field.
        serde_json::Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if names_a_secret(key) {
                    blank_strings(child);
                } else {
                    redact_url_strings(child);
                }
            }
        }
        _ => {}
    }
}

fn names_a_url(key: &str) -> bool {
    // `url` alone or as a suffix, so a key such as `curl` is not a URL.
    let key = key.to_ascii_lowercase();
    key == "url"
        || ["_url", "endpoint", "_dsn"]
            .iter()
            .any(|suffix| key.ends_with(suffix))
}

fn names_a_secret(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "password",
        "secret",
        "token",
        "api_key",
        "app_key",
        "encrypt_key",
        "pair_code",
    ]
    .iter()
    .any(|suffix| key.ends_with(suffix))
}

/// Serializes the client-facing AI config slice consumed by the settings UI.
pub async fn client_config_json(config: &Config) -> serde_json::Value {
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
                "ca_cert_pem": config.cloud_provider_ca_certs.get(&c.slug),
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
    let credits_bypass_chat =
        crate::inference::provider::factory::role_bypasses_managed_credits("chat", config).await;
    let credits_bypass_reasoning =
        crate::inference::provider::factory::role_bypasses_managed_credits("reasoning", config)
            .await;

    serde_json::json!({
        "api_url": config.api_url,
        "inference_url": config.inference_url,
        "default_model": config.default_model,
        // The composer's thinking-level default (`runtime.reasoning_effort`);
        // `null` means the provider decides.
        "reasoning_effort": config.runtime.reasoning_effort,
        // Per-model thinking levels (`runtime.reasoning_effort_by_model`).
        "reasoning_effort_by_model": config.runtime.reasoning_effort_by_model,
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
            "chat": credits_bypass_chat,
            "reasoning": credits_bypass_reasoning,
        },
        "chat_provider": config.chat_provider,
        "reasoning_provider": config.reasoning_provider,
        "agentic_provider": config.agentic_provider,
        "coding_provider": config.coding_provider,
        "vision_provider": config.vision_provider,
        "memory_provider": config.memory_provider,
        "embeddings_provider": config.embeddings_provider,
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
pub async fn load_and_get_client_config_snapshot(
    ctx: &DispatchCtx,
) -> Result<Outcome<serde_json::Value>, String> {
    let config = ctx.load_config().await?;
    let snapshot = client_config_json(&config).await;
    Ok(Outcome::new(
        snapshot,
        vec!["client config read".to_string()],
    ))
}

/// Returns a full configuration snapshot for the UI.
pub async fn get_config_snapshot(config: &Config) -> Result<Outcome<serde_json::Value>, String> {
    let snapshot = snapshot_config_json(config).await?;
    Ok(Outcome::new(
        snapshot,
        vec![format!(
            "config loaded from {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration from disk and returns a snapshot.
pub async fn load_and_get_config_snapshot(
    ctx: &DispatchCtx,
) -> Result<Outcome<serde_json::Value>, String> {
    let config = ctx.load_config().await?;
    get_config_snapshot(&config).await
}

/// Reads dashboard settings exposed to the desktop UI.
pub async fn get_dashboard_settings(
    ctx: &DispatchCtx,
) -> Result<Outcome<serde_json::Value>, String> {
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

    let config = ctx.load_config().await.map_err(|error| {
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
    Ok(Outcome::new(
        result,
        vec!["dashboard settings read".to_string()],
    ))
}
