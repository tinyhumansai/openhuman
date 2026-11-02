//! TinyHumans host implementation of the core provider credential port.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::credentials::{
    ProviderCredentialSource, ProviderOAuthStart, ProviderOAuthTokens,
};
use std::path::PathBuf;
use tinyinference_providers::oauth::{
    openai_codex_config, parse_openai_codex_auth_json, refresh_access_token, OAuthFlow,
    OAuthTokenSet,
};

use super::pending;

const REDIRECT_URI: &str = "http://127.0.0.1:1455/auth/callback";

#[derive(Debug, Default)]
pub(crate) struct OpenAiProviderCredentialSource {
    #[cfg(test)]
    pub(crate) token_url_override: Option<String>,
}

async fn flow(config: &Config) -> Result<OAuthFlow, String> {
    let (state_dir, secrets) = pending::store(config)?;
    pending::adopt_legacy(&state_dir, &secrets).await;
    Ok(OAuthFlow::new(openai_codex_config(REDIRECT_URI), secrets))
}

#[async_trait]
impl ProviderCredentialSource for OpenAiProviderCredentialSource {
    async fn start_openai_oauth(&self, config: &Config) -> Result<ProviderOAuthStart, String> {
        let flow = flow(config).await?;
        let start = flow.start().await?;
        Ok(ProviderOAuthStart {
            auth_url: start.auth_url,
            state: start.state,
            redirect_uri: start.redirect_uri,
        })
    }

    async fn complete_openai_oauth(
        &self,
        config: &Config,
        callback: &str,
    ) -> Result<ProviderOAuthTokens, String> {
        let flow = flow(config).await?;
        let token = flow
            .complete(callback)
            .await
            .map_err(|error| error.to_string())?;
        Ok(ProviderOAuthTokens {
            token,
            account_id: None,
        })
    }

    fn import_openai_oauth_from_codex_cli(
        &self,
        _config: &Config,
    ) -> Result<ProviderOAuthTokens, String> {
        let path = codex_cli_auth_path()?;
        log::info!(
            "[provider-oauth] Codex CLI import started path={}",
            path.display()
        );
        let bytes = std::fs::read(&path).map_err(|error| {
            format!(
                "Could not read Codex CLI auth at {}: {error}. Run `codex login` first, then try Codex auth again.",
                path.display()
            )
        })?;
        parse_codex_cli_auth(&bytes).map_err(|error| {
            format!(
                "Could not parse Codex CLI auth at {}: {error}. Run `codex login` again, then try Codex auth again.",
                path.display()
            )
        })
    }

    fn resolve_openai_oauth(
        &self,
        provider: &str,
        refresh_token: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<Option<OAuthTokenSet>, String> {
        if provider != "provider:openai" && provider != "openai" {
            return Err(format!("provider OAuth is unsupported for {provider}"));
        }
        let Some(expires_at) = expires_at else {
            return Ok(None);
        };
        if expires_at > Utc::now() + Duration::minutes(2) {
            return Ok(None);
        }
        let config = openai_codex_config(REDIRECT_URI);
        #[cfg(test)]
        let config = if let Some(token_url) = self
            .token_url_override
            .clone()
            .filter(|endpoint| is_loopback_http_endpoint(endpoint))
        {
            let mut config = config;
            config.token_url = token_url;
            config
        } else {
            config
        };
        let refresh_token = refresh_token.to_string();
        if tokio::runtime::Handle::try_current().is_err() {
            return Err("tokio runtime required to refresh openai oauth token".to_string());
        }
        let refreshed = std::thread::Builder::new()
            .name("openai-oauth-refresh".to_string())
            .spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| error.to_string())?
                    .block_on(refresh_access_token(&config, &refresh_token))
                    .map_err(|error| error.to_string())
            })
            .map_err(|error| error.to_string())?
            .join()
            .map_err(|_| "OpenAI OAuth refresh worker panicked".to_string())?;
        let refreshed = match refreshed {
            Ok(refreshed) => refreshed,
            Err(error) => {
                log::warn!("[provider-oauth] OpenAI OAuth refresh failed: {error}");
                if expires_at <= Utc::now() {
                    // Preserve the phrase consumed by core's inference error
                    // classifier so this routes to Codex reconnect guidance.
                    return Err(format!(
                        "Codex authentication token is expired — refresh failed: {error}. \
                         Please reconnect Codex in Settings → Integrations."
                    ));
                }
                return Ok(None);
            }
        };
        if refreshed.access_token.trim().is_empty() {
            return Err("OAuth refresh response contained an empty access token".to_owned());
        }
        Ok(Some(refreshed))
    }

    async fn clear_openai_oauth_pending(&self, config: &Config) -> Result<(), String> {
        pending::clear(config).await
    }
}

pub(super) fn parse_codex_cli_auth(bytes: &[u8]) -> Result<ProviderOAuthTokens, String> {
    let imported = parse_openai_codex_auth_json(bytes)?;
    Ok(ProviderOAuthTokens {
        token: imported.token,
        account_id: imported.account_id,
    })
}

fn codex_cli_auth_path() -> Result<PathBuf, String> {
    if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
        let path = PathBuf::from(codex_home);
        if !path.as_os_str().is_empty() {
            return Ok(path.join("auth.json"));
        }
    }
    let home = home_dir_from_env()
        .ok_or_else(|| "home directory is not set; cannot find ~/.codex/auth.json".to_string())?;
    Ok(home.join(".codex").join("auth.json"))
}

fn home_dir_from_env() -> Option<PathBuf> {
    for key in ["HOME", "USERPROFILE"] {
        if let Some(value) = std::env::var_os(key) {
            let path = PathBuf::from(value);
            if !path.as_os_str().is_empty() {
                return Some(path);
            }
        }
    }
    match (std::env::var_os("HOMEDRIVE"), std::env::var_os("HOMEPATH")) {
        (Some(drive), Some(path))
            if !drive.as_os_str().is_empty() && !path.as_os_str().is_empty() =>
        {
            Some(PathBuf::from(drive).join(path))
        }
        _ => None,
    }
}

#[cfg(test)]
pub(super) fn is_loopback_http_endpoint(endpoint: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(endpoint) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    parsed.scheme() == "http" && matches!(host, "localhost" | "127.0.0.1" | "::1")
}
