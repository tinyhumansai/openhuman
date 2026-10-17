//! Module-owned connection preparation, followed by host credential custody
//! and configuration persistence.
use super::shared::credential_provider;
use crate::config::Config;
use crate::core::Outcome;
use crate::security::credentials;
use serde_json::Value;
use tinybus::module::ModuleCallError;
use tinychannels_bus::controllers::ChannelConnectionResult;
use tinychannels_bus::module_controller::{ChannelConfigUpdate, PreparedChannelConnection};
use tinychannels_bus::{ChannelAuthMode, ChannelControllerRequest, HttpProxyPolicy};

/// Validate and verify through TinyChannels, then persist the approved provider
/// update and credentials in OpenHuman's existing stores.
pub async fn connect_channel(
    config: &Config,
    channel_id: &str,
    auth_mode: ChannelAuthMode,
    credentials_value: Value,
) -> Result<Outcome<ChannelConnectionResult>, String> {
    let service = format!("channel.{channel_id}");
    let proxy = if config.proxy.should_apply_to_service(&service) {
        HttpProxyPolicy {
            http_proxy: config.proxy.http_proxy.clone(),
            https_proxy: config.proxy.https_proxy.clone(),
            all_proxy: config.proxy.all_proxy.clone(),
            no_proxy: config.proxy.normalized_no_proxy(),
            service_selectors: vec![service],
        }
    } else {
        HttpProxyPolicy::default()
    };
    let prepared = crate::modules::channel_controller::prepare_connection(
        &crate::modules::client(config.clone()),
        ChannelControllerRequest::PrepareConnect {
            config: config.channels_config.clone(),
            channel: channel_id.to_owned(),
            auth_mode,
            credentials: credentials_value.clone(),
            proxy,
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    persist_prepared_connection(config, channel_id, auth_mode, credentials_value, prepared).await
}

pub(crate) async fn persist_prepared_connection(
    config: &Config,
    channel_id: &str,
    auth_mode: ChannelAuthMode,
    credentials_value: Value,
    prepared: PreparedChannelConnection,
) -> Result<Outcome<ChannelConnectionResult>, String> {
    if prepared.result.status == "pending_auth"
        && prepared.result.auth_action.is_some()
        && prepared.config_update.is_none()
        && !prepared.result.restart_required
    {
        return Ok(Outcome::new(prepared.result, vec![]));
    }
    if prepared.result.status != "connected"
        || prepared.result.auth_action.is_some()
        || !update_matches_scope(channel_id, auth_mode, prepared.config_update.as_ref())
    {
        return Err(malformed());
    }
    let creds_map = credentials_value.as_object().ok_or_else(malformed)?;
    let local_only = channel_id == "imessage" && auth_mode == ChannelAuthMode::ManagedDm;
    let provider_key = credential_provider(channel_id, auth_mode);
    if !local_only {
        // Credential storage policy remains host-owned. Provider validation has
        // already succeeded in the module before any store is touched.
        let token = creds_map
            .get("bot_token")
            .or_else(|| creds_map.get("api_key"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let fields = if creds_map.len() > 1 || (creds_map.len() == 1 && token.is_none()) {
            Some(Value::Object(creds_map.clone()))
        } else {
            None
        };
        credentials::ops::store_provider_credentials(
            config,
            &provider_key,
            None,
            token,
            fields,
            Some(true),
        )
        .await
        .map_err(|error| format!("failed to store credentials: {error}"))?;
    }
    if let Some(update) = prepared.config_update {
        if let ChannelConfigUpdate::Email(value) = update {
            super::email::persist_email_config(config, value).await?;
        } else {
            let mut persisted = config.clone();
            match update {
                ChannelConfigUpdate::Telegram(value) => {
                    persisted.channels_config.telegram = Some(value)
                }
                ChannelConfigUpdate::Discord(value) => {
                    persisted.channels_config.discord = Some(value)
                }
                ChannelConfigUpdate::IMessage(value) => {
                    persisted.channels_config.imessage = Some(value)
                }
                ChannelConfigUpdate::Email(_) => unreachable!("email persistence handled above"),
                ChannelConfigUpdate::Yuanbao(mut value) => {
                    value.app_secret.clear();
                    persisted.channels_config.yuanbao = Some(*value);
                }
            }
            persisted
                .save()
                .await
                .map_err(|error| format!("failed to persist {channel_id} config.toml: {error}"))?;
        }
    }
    let log = if local_only {
        "stored imessage channel config (local-only)".to_owned()
    } else {
        format!("stored credentials for {provider_key}")
    };
    Ok(Outcome::single_log(prepared.result, log))
}

fn update_matches_scope(
    channel: &str,
    mode: ChannelAuthMode,
    update: Option<&ChannelConfigUpdate>,
) -> bool {
    match (channel, mode, update) {
        ("telegram", ChannelAuthMode::BotToken, Some(ChannelConfigUpdate::Telegram(_)))
        | ("discord", ChannelAuthMode::BotToken, Some(ChannelConfigUpdate::Discord(_)))
        | ("imessage", ChannelAuthMode::ManagedDm, Some(ChannelConfigUpdate::IMessage(_)))
        | ("email", ChannelAuthMode::ApiKey, Some(ChannelConfigUpdate::Email(_)))
        | ("yuanbao", ChannelAuthMode::ApiKey, Some(ChannelConfigUpdate::Yuanbao(_))) => true,
        ("telegram", ChannelAuthMode::BotToken, _)
        | ("discord", ChannelAuthMode::BotToken, _)
        | ("imessage", ChannelAuthMode::ManagedDm, _)
        | ("email", ChannelAuthMode::ApiKey, _)
        | ("yuanbao", ChannelAuthMode::ApiKey, _) => false,
        (_, _, None) => true,
        _ => false,
    }
}

fn malformed() -> String {
    crate::modules::report::malformed_reply("tinychannels");
    ModuleCallError::ModuleFault.to_string()
}

#[cfg(test)]
#[path = "connect_channel_tests.rs"]
mod tests;
