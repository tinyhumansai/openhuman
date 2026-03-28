use serde_json::json;

use crate::core_server::helpers::{
    auth_service_from_config, build_session_state, get_session_token, parse_fields_value,
    parse_params, profile_name_or_default, summarize_auth_profile,
};
use crate::core_server::types::{
    AuthListProviderCredentialsParams, AuthRemoveProviderCredentialsParams,
    AuthStoreProviderCredentialsParams, AuthStoreSessionParams, InvocationResult, SocketConnectParams,
    SocketEmitParams,
};
use crate::core_server::APP_SESSION_PROVIDER;
use crate::openhuman::config::Config;

pub async fn try_dispatch(
    method: &str,
    params: serde_json::Value,
) -> Option<Result<InvocationResult, String>> {
    match method {
        "openhuman.auth.store_session" => Some(
            async move {
                let config = Config::load_or_init().await.map_err(|e| e.to_string())?;
                let payload: AuthStoreSessionParams = parse_params(params)?;
                let trimmed_token = payload.token.trim();
                if trimmed_token.is_empty() {
                    return Err("token is required".to_string());
                }

                let mut metadata = std::collections::HashMap::new();
                if let Some(user_id) = payload.user_id.and_then(|v| {
                    let t = v.trim().to_string();
                    (!t.is_empty()).then_some(t)
                }) {
                    metadata.insert("user_id".to_string(), user_id);
                }
                if let Some(user) = payload.user {
                    metadata.insert("user_json".to_string(), user.to_string());
                }

                let auth = auth_service_from_config(&config);
                let profile = auth
                    .store_provider_token(
                        APP_SESSION_PROVIDER,
                        crate::core_server::DEFAULT_AUTH_PROFILE_NAME,
                        trimmed_token,
                        metadata,
                        true,
                    )
                    .map_err(|e| e.to_string())?;

                InvocationResult::with_logs(
                    summarize_auth_profile(&profile),
                    vec!["session stored".to_string()],
                )
            }
            .await,
        ),

        "openhuman.auth.clear_session" => Some(
            async move {
                let config = Config::load_or_init().await.map_err(|e| e.to_string())?;
                let auth = auth_service_from_config(&config);
                let removed = auth
                    .remove_profile(
                        APP_SESSION_PROVIDER,
                        crate::core_server::DEFAULT_AUTH_PROFILE_NAME,
                    )
                    .map_err(|e| e.to_string())?;
                InvocationResult::with_logs(
                    json!({ "removed": removed }),
                    vec!["session cleared".to_string()],
                )
            }
            .await,
        ),

        "openhuman.auth.get_state" => Some(
            async move {
                let config = Config::load_or_init().await.map_err(|e| e.to_string())?;
                let state = build_session_state(&config)?;
                InvocationResult::with_logs(state, vec!["session state fetched".to_string()])
            }
            .await,
        ),

        "openhuman.auth.get_session_token" => Some(
            async move {
                let config = Config::load_or_init().await.map_err(|e| e.to_string())?;
                let token = get_session_token(&config)?;
                InvocationResult::with_logs(
                    json!({ "token": token }),
                    vec!["session token fetched".to_string()],
                )
            }
            .await,
        ),

        "openhuman.auth.store_provider_credentials" => Some(
            async move {
                let config = Config::load_or_init().await.map_err(|e| e.to_string())?;
                let payload: AuthStoreProviderCredentialsParams = parse_params(params)?;
                let provider = payload.provider.trim().to_string();
                if provider.is_empty() {
                    return Err("provider is required".to_string());
                }

                let profile_name = profile_name_or_default(payload.profile.as_deref());
                let mut metadata = parse_fields_value(payload.fields)?;
                let token = payload
                    .token
                    .as_ref()
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
                    .or_else(|| metadata.get("token").cloned())
                    .or_else(|| metadata.get("api_key").cloned())
                    .unwrap_or_default();
                if token.is_empty() && metadata.is_empty() {
                    return Err("provide at least one credential via token or fields".to_string());
                }
                metadata.remove("token");

                let auth = auth_service_from_config(&config);
                let profile = auth
                    .store_provider_token(
                        &provider,
                        profile_name,
                        &token,
                        metadata,
                        payload.set_active.unwrap_or(true),
                    )
                    .map_err(|e| e.to_string())?;
                InvocationResult::with_logs(
                    summarize_auth_profile(&profile),
                    vec!["provider credentials stored".to_string()],
                )
            }
            .await,
        ),

        "openhuman.auth.remove_provider_credentials" => Some(
            async move {
                let config = Config::load_or_init().await.map_err(|e| e.to_string())?;
                let payload: AuthRemoveProviderCredentialsParams = parse_params(params)?;
                let profile_name = profile_name_or_default(payload.profile.as_deref());
                let auth = auth_service_from_config(&config);
                let removed = auth
                    .remove_profile(&payload.provider, profile_name)
                    .map_err(|e| e.to_string())?;
                InvocationResult::with_logs(
                    json!({ "removed": removed, "provider": payload.provider, "profile": profile_name }),
                    vec!["provider credentials removed".to_string()],
                )
            }
            .await,
        ),

        "openhuman.auth.list_provider_credentials" => Some(
            async move {
                let config = Config::load_or_init().await.map_err(|e| e.to_string())?;
                let payload: AuthListProviderCredentialsParams = if params.is_null() {
                    AuthListProviderCredentialsParams { provider: None }
                } else {
                    parse_params(params)?
                };
                let provider_filter = payload
                    .provider
                    .as_deref()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_string);

                let auth = auth_service_from_config(&config);
                let profiles = auth.load_profiles().map_err(|e| e.to_string())?;
                let mut items = profiles
                    .profiles
                    .values()
                    .filter(|profile| profile.provider != APP_SESSION_PROVIDER)
                    .filter(|profile| {
                        provider_filter
                            .as_ref()
                            .is_none_or(|provider| profile.provider == *provider)
                    })
                    .map(summarize_auth_profile)
                    .collect::<Vec<_>>();
                items.sort_by(|a, b| {
                    a.provider
                        .cmp(&b.provider)
                        .then_with(|| a.profile_name.cmp(&b.profile_name))
                });

                InvocationResult::with_logs(
                    items,
                    vec!["provider credentials listed".to_string()],
                )
            }
            .await,
        ),

        "openhuman.socket.connect" => Some(
            async move {
                let payload: SocketConnectParams = parse_params(params)?;
                #[cfg(feature = "tauri-host")]
                {
                    let mgr = crate::core_server::helpers::core_socket_manager();
                    mgr.connect(&payload.url, &payload.token).await?;
                    return InvocationResult::with_logs(
                        mgr.get_state(),
                        vec!["socket connect requested".to_string()],
                    );
                }
                #[cfg(not(feature = "tauri-host"))]
                {
                    let _ = payload;
                    Err("socket runtime is unavailable in this build".to_string())
                }
            }
            .await,
        ),

        "openhuman.socket.disconnect" => Some(
            async move {
                #[cfg(feature = "tauri-host")]
                {
                    let mgr = crate::core_server::helpers::core_socket_manager();
                    mgr.disconnect().await?;
                    return InvocationResult::with_logs(
                        mgr.get_state(),
                        vec!["socket disconnected".to_string()],
                    );
                }
                #[cfg(not(feature = "tauri-host"))]
                {
                    Err("socket runtime is unavailable in this build".to_string())
                }
            }
            .await,
        ),

        "openhuman.socket.state" => Some(
            async move {
                #[cfg(feature = "tauri-host")]
                {
                    let mgr = crate::core_server::helpers::core_socket_manager();
                    return InvocationResult::with_logs(
                        mgr.get_state(),
                        vec!["socket state fetched".to_string()],
                    );
                }
                #[cfg(not(feature = "tauri-host"))]
                {
                    Err("socket runtime is unavailable in this build".to_string())
                }
            }
            .await,
        ),

        "openhuman.socket.emit" => Some(
            async move {
                let payload: SocketEmitParams = parse_params(params)?;
                #[cfg(feature = "tauri-host")]
                {
                    let mgr = crate::core_server::helpers::core_socket_manager();
                    mgr.emit(
                        &payload.event,
                        payload.data.unwrap_or(serde_json::Value::Null),
                    )
                    .await?;
                    return InvocationResult::with_logs(
                        mgr.get_state(),
                        vec!["socket event emitted".to_string()],
                    );
                }
                #[cfg(not(feature = "tauri-host"))]
                {
                    let _ = payload;
                    Err("socket runtime is unavailable in this build".to_string())
                }
            }
            .await,
        ),

        _ => None,
    }
}
