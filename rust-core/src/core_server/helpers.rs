use serde::de::DeserializeOwned;
use std::path::PathBuf;

#[cfg(feature = "tauri-host")]
use std::sync::{Arc, OnceLock};

use crate::auth::AuthService;
use crate::auth::profiles::{AuthProfileKind, TokenSet};
use crate::openhuman::config::Config;
use crate::openhuman::security::SecretStore;

use super::types::{
    AuthProfileSummary, AuthStateResponse,
};
use super::{APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};

#[cfg(feature = "tauri-host")]
static SOCKET_MANAGER: OnceLock<Arc<crate::runtime::socket_manager::SocketManager>> =
    OnceLock::new();

#[cfg(feature = "tauri-host")]
pub fn core_socket_manager() -> Arc<crate::runtime::socket_manager::SocketManager> {
    SOCKET_MANAGER
        .get_or_init(|| Arc::new(crate::runtime::socket_manager::SocketManager::new()))
        .clone()
}

pub async fn load_openhuman_config() -> Result<Config, String> {
    let timeout_duration = std::time::Duration::from_secs(30);
    match tokio::time::timeout(timeout_duration, Config::load_or_init()).await {
        Ok(Ok(config)) => Ok(config),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("Config loading timed out".to_string()),
    }
}

pub fn snapshot_config(config: &Config) -> Result<super::types::ConfigSnapshot, String> {
    let value = serde_json::to_value(config).map_err(|e| e.to_string())?;
    Ok(super::types::ConfigSnapshot {
        config: value,
        workspace_dir: config.workspace_dir.display().to_string(),
        config_path: config.config_path.display().to_string(),
    })
}

pub fn env_flag_enabled(key: &str) -> bool {
    matches!(
        std::env::var(key).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

pub fn core_rpc_url() -> String {
    std::env::var("OPENHUMAN_CORE_RPC_URL")
        .unwrap_or_else(|_| super::DEFAULT_CORE_RPC_URL.to_string())
}

pub fn default_workspace_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".openhuman")
        .join("workspace")
}

pub fn secret_store_for_config(config: &Config) -> SecretStore {
    let data_dir = config
        .config_path
        .parent()
        .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
    SecretStore::new(&data_dir, true)
}

pub fn parse_params<T: DeserializeOwned>(params: serde_json::Value) -> Result<T, String> {
    serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))
}

pub fn extract_namespaces_from_documents(payload: &serde_json::Value) -> Vec<String> {
    fn collect_from_value(value: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(ns) = map.get("namespace").and_then(serde_json::Value::as_str) {
                    if !ns.trim().is_empty() {
                        out.insert(ns.to_string());
                    }
                }
                for nested in map.values() {
                    collect_from_value(nested, out);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    collect_from_value(item, out);
                }
            }
            _ => {}
        }
    }

    let mut namespaces = std::collections::BTreeSet::new();
    collect_from_value(payload, &mut namespaces);
    namespaces.into_iter().collect()
}

pub fn filter_documents_payload_by_namespace(
    payload: serde_json::Value,
    namespace: &str,
) -> serde_json::Value {
    fn filter_array(items: &mut Vec<serde_json::Value>, namespace: &str) {
        items.retain(|item| {
            item.as_object()
                .and_then(|obj| obj.get("namespace"))
                .and_then(serde_json::Value::as_str)
                .map(|ns| ns == namespace)
                .unwrap_or(false)
        });
    }

    match payload {
        serde_json::Value::Array(mut items) => {
            filter_array(&mut items, namespace);
            serde_json::Value::Array(items)
        }
        serde_json::Value::Object(mut root) => {
            for key in ["documents", "items", "results"] {
                if let Some(serde_json::Value::Array(items)) = root.get_mut(key) {
                    filter_array(items, namespace);
                    return serde_json::Value::Object(root);
                }
            }

            if let Some(serde_json::Value::Object(data)) = root.get_mut("data") {
                for key in ["documents", "items", "results"] {
                    if let Some(serde_json::Value::Array(items)) = data.get_mut(key) {
                        filter_array(items, namespace);
                        return serde_json::Value::Object(root);
                    }
                }
            }

            serde_json::Value::Object(root)
        }
        other => other,
    }
}

pub fn auth_service_from_config(config: &Config) -> AuthService {
    AuthService::from_config(config)
}

pub fn profile_name_or_default(value: Option<&str>) -> &str {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_AUTH_PROFILE_NAME)
}

pub fn parse_fields_value(
    input: Option<serde_json::Value>,
) -> Result<std::collections::HashMap<String, String>, String> {
    let Some(value) = input else {
        return Ok(std::collections::HashMap::new());
    };

    let Some(map) = value.as_object() else {
        return Err("fields must be a JSON object".to_string());
    };

    let mut out = std::collections::HashMap::new();
    for (key, raw) in map {
        if key.trim().is_empty() {
            return Err("fields cannot contain empty keys".to_string());
        }
        let rendered = match raw {
            serde_json::Value::Null => String::new(),
            serde_json::Value::String(s) => s.clone(),
            _ => raw.to_string(),
        };
        out.insert(key.clone(), rendered);
    }

    Ok(out)
}

fn profile_kind_label(kind: AuthProfileKind) -> String {
    match kind {
        AuthProfileKind::OAuth => "oauth".to_string(),
        AuthProfileKind::Token => "token".to_string(),
    }
}

pub fn summarize_auth_profile(profile: &crate::auth::profiles::AuthProfile) -> AuthProfileSummary {
    let mut metadata_keys = profile
        .metadata
        .keys()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>();
    metadata_keys.sort();

    AuthProfileSummary {
        id: profile.id.clone(),
        provider: profile.provider.clone(),
        profile_name: profile.profile_name.clone(),
        kind: profile_kind_label(profile.kind),
        account_id: profile.account_id.clone(),
        workspace_id: profile.workspace_id.clone(),
        metadata_keys,
        updated_at: profile.updated_at.to_rfc3339(),
        has_token: profile.token.as_ref().is_some_and(|v| !v.trim().is_empty()),
        has_token_set: profile
            .token_set
            .as_ref()
            .map(|TokenSet { access_token, .. }| !access_token.trim().is_empty())
            .unwrap_or(false),
    }
}

fn session_user_value(profile: &crate::auth::profiles::AuthProfile) -> Option<serde_json::Value> {
    profile
        .metadata
        .get("user_json")
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
}

pub fn build_session_state(config: &Config) -> Result<AuthStateResponse, String> {
    let auth_service = auth_service_from_config(config);
    let profile = auth_service
        .get_profile(APP_SESSION_PROVIDER, None)
        .map_err(|e| e.to_string())?;

    let Some(profile) = profile else {
        return Ok(AuthStateResponse {
            is_authenticated: false,
            user_id: None,
            user: None,
            profile_id: None,
        });
    };

    let is_authenticated = profile
        .token
        .as_ref()
        .map(|token| !token.trim().is_empty())
        .unwrap_or(false);

    Ok(AuthStateResponse {
        is_authenticated,
        user_id: profile.metadata.get("user_id").cloned(),
        user: session_user_value(&profile),
        profile_id: Some(profile.id),
    })
}

pub fn get_session_token(config: &Config) -> Result<Option<String>, String> {
    let auth_service = auth_service_from_config(config);
    let profile = auth_service
        .get_profile(APP_SESSION_PROVIDER, None)
        .map_err(|e| e.to_string())?;
    Ok(profile.and_then(|entry| entry.token))
}
