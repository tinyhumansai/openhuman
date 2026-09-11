//! Core CLI auth flows: load config, branch `app-session` vs provider storage.

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::security::credentials::rpc;
use crate::openhuman::security::credentials::APP_SESSION_PROVIDER;

pub fn parse_field_equals_entries(entries: &[String]) -> Result<serde_json::Value, String> {
    let mut fields = serde_json::Map::new();
    for entry in entries {
        let Some((raw_key, raw_value)) = entry.split_once('=') else {
            return Err(format!(
                "invalid --field value '{entry}', expected key=value format"
            ));
        };
        let key = raw_key.trim();
        if key.is_empty() {
            return Err("invalid --field value with empty key".to_string());
        }
        fields.insert(
            key.to_string(),
            serde_json::Value::String(raw_value.to_string()),
        );
    }
    Ok(serde_json::Value::Object(fields))
}

pub async fn cli_auth_login(
    provider: String,
    token: String,
    user_id: Option<String>,
    user_json: Option<serde_json::Value>,
    fields: serde_json::Value,
    profile: Option<String>,
    set_active: bool,
) -> Result<serde_json::Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let provider = provider.trim().to_string();

    if provider == APP_SESSION_PROVIDER {
        rpc::store_session(&config, &token, user_id, user_json)
            .await?
            .into_cli_compatible_json()
    } else {
        let fields_opt = match &fields {
            serde_json::Value::Object(map) if map.is_empty() => None,
            _ => Some(fields),
        };
        rpc::store_provider_credentials(
            &config,
            &provider,
            profile.as_deref(),
            Some(token),
            fields_opt,
            Some(set_active),
        )
        .await?
        .into_cli_compatible_json()
    }
}

pub async fn cli_auth_logout(
    provider: String,
    profile: Option<String>,
) -> Result<serde_json::Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let provider = provider.trim().to_string();
    if provider == APP_SESSION_PROVIDER {
        rpc::clear_session(&config)
            .await?
            .into_cli_compatible_json()
    } else {
        rpc::remove_provider_credentials(&config, &provider, profile.as_deref())
            .await?
            .into_cli_compatible_json()
    }
}

pub async fn cli_auth_status(
    provider: String,
    _profile: Option<String>,
) -> Result<serde_json::Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let provider = provider.trim().to_string();
    if provider == APP_SESSION_PROVIDER {
        rpc::auth_get_state(&config)
            .await?
            .into_cli_compatible_json()
    } else {
        rpc::list_provider_credentials(&config, Some(provider))
            .await?
            .into_cli_compatible_json()
    }
}

pub async fn cli_auth_list(provider_filter: Option<String>) -> Result<serde_json::Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let filter = provider_filter
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    rpc::list_provider_credentials(&config, filter)
        .await?
        .into_cli_compatible_json()
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
