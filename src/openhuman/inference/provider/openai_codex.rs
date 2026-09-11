use std::path::{Path, PathBuf};

use crate::openhuman::config::Config;

pub(crate) const OPENAI_CODEX_ACCOUNT_HEADER: &str = "ChatGPT-Account-ID";
pub(crate) const OPENAI_CODEX_BACKEND_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
pub(crate) const OPENAI_CODEX_ORIGINATOR_HEADER: &str = "originator";
pub(crate) const OPENAI_CODEX_ORIGINATOR: &str = "codex_cli_rs";
pub(crate) const OPENAI_CODEX_MODEL_HINTS: &[&str] =
    &["gpt-5.5", "gpt-5.4", "gpt-5.3-codex-spark", "gpt-5.3-codex"];
// Conservative Codex CLI release known to work with the ChatGPT Codex backend.
// Bump this when field reports show the backend rejecting older client versions.
const OPENAI_CODEX_DEFAULT_CLIENT_VERSION: &str = "0.130.0";

pub(crate) fn openai_codex_user_agent() -> String {
    format!(
        "codex_cli_rs/0.0.0 (OpenHuman {})",
        env!("CARGO_PKG_VERSION")
    )
}

#[derive(Debug, Clone)]
pub(crate) struct OpenAiCodexRouting {
    pub endpoint: String,
    pub using_oauth: bool,
    pub account_id: Option<String>,
}

impl OpenAiCodexRouting {
    pub fn standard(endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            using_oauth: false,
            account_id: None,
        }
    }
}

pub(crate) fn openai_codex_client_version() -> String {
    log::trace!("[providers][openai-codex] client_version resolve start");
    let (version, source) = resolve_openai_codex_client_version();
    log::debug!(
        "[providers][openai-codex] resolved client_version source={source} value={version}"
    );
    version
}

fn resolve_openai_codex_client_version() -> (String, &'static str) {
    if let Some(version) = std::env::var("OPENAI_CODEX_CLIENT_VERSION")
        .ok()
        .and_then(non_empty_trimmed)
    {
        log::trace!("[providers][openai-codex] client_version source=env");
        return (version, "env");
    }

    if let Some(home) = codex_home_dir() {
        log::trace!(
            "[providers][openai-codex] client_version probing codex home={}",
            home.display()
        );
        if let Some(version) =
            read_json_string_field(&home.join("models_cache.json"), "client_version")
        {
            log::trace!("[providers][openai-codex] client_version source=models_cache");
            return (version, "models_cache");
        }
        if let Some(version) = read_json_string_field(&home.join("version.json"), "latest_version")
        {
            log::trace!("[providers][openai-codex] client_version source=version_json");
            return (version, "version_json");
        }
    } else {
        log::trace!("[providers][openai-codex] client_version codex home unresolved");
    }

    log::trace!("[providers][openai-codex] client_version source=default");
    (OPENAI_CODEX_DEFAULT_CLIENT_VERSION.to_string(), "default")
}

fn non_empty_trimmed(value: String) -> Option<String> {
    let trimmed = value.trim().to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn codex_home_dir() -> Option<PathBuf> {
    if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
        let path = PathBuf::from(codex_home);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }

    home_dir_from_env().map(|home| home.join(".codex"))
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

fn read_json_string_field(path: &Path, field: &str) -> Option<String> {
    let file = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("<unknown>");
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            log::trace!("[providers][openai-codex] client_version read miss file={file} err={err}");
            return None;
        }
    };
    let json: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(json) => json,
        Err(err) => {
            log::trace!(
                "[providers][openai-codex] client_version parse miss file={file} err={err}"
            );
            return None;
        }
    };
    json.get(field)
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .and_then(non_empty_trimmed)
}

pub(crate) fn resolve_openai_codex_routing(
    config: &Config,
    slug: &str,
    endpoint: &str,
    bearer_key: &str,
    bearer_is_oauth: bool,
) -> Result<OpenAiCodexRouting, String> {
    if slug != "openai" {
        return Ok(OpenAiCodexRouting::standard(endpoint));
    }

    let credentials =
        match crate::openhuman::inference::openai_oauth::lookup_openai_oauth_credentials(config) {
            Ok(credentials) => credentials,
            Err(err) if !bearer_key.trim().is_empty() => {
                log::warn!(
                    "[providers][openai-codex] oauth metadata unavailable; continuing with standard bearer key: {err}"
                );
                None
            }
            Err(err) => return Err(format!("[chat-factory] openai oauth lookup failed: {err}")),
        };

    // Route to the Codex subscription backend when the resolved openai bearer is
    // an OAuth (Codex-CLI) credential — decided by the effective profile's *kind*
    // (`bearer_is_oauth`, from the credential store), not by string-comparing two
    // independently resolved OAuth tokens. The old
    // `credentials.access_token == bearer_key` check silently fell back to
    // api.openai.com and failed with HTTP 400 "Stream must be set to true"
    // (#5353): each read of the OAuth credential can trigger a refresh, OpenAI
    // rotates the access token on every refresh, so the two reads diverged
    // whenever they straddled a refresh — even though OAuth was the only cred.
    // Still require resolved `credentials` so we have the account id / endpoint;
    // if the metadata lookup failed we fall back to the standard path.
    let using_oauth = bearer_is_oauth && credentials.is_some();
    let account_id = credentials
        .filter(|_| using_oauth)
        .and_then(|credentials| credentials.account_id)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    Ok(if using_oauth {
        OpenAiCodexRouting {
            endpoint: OPENAI_CODEX_BACKEND_BASE_URL.to_string(),
            using_oauth: true,
            account_id,
        }
    } else {
        OpenAiCodexRouting::standard(endpoint)
    })
}

#[cfg(test)]
#[path = "openai_codex_tests.rs"]
mod tests;
