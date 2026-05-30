use crate::openhuman::config::Config;

pub(crate) const OPENAI_CODEX_ACCOUNT_HEADER: &str = "ChatGPT-Account-ID";
pub(crate) const OPENAI_CODEX_BACKEND_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
pub(crate) const OPENAI_CODEX_ORIGINATOR_HEADER: &str = "originator";
pub(crate) const OPENAI_CODEX_ORIGINATOR: &str = "codex_cli_rs";
pub(crate) const OPENAI_CODEX_MODEL_HINTS: &[&str] =
    &["gpt-5.5", "gpt-5.4", "gpt-5.3-codex-spark", "gpt-5.3-codex"];

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

pub(crate) fn openai_codex_client_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub(crate) fn resolve_openai_codex_routing(
    config: &Config,
    slug: &str,
    endpoint: &str,
    bearer_key: &str,
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

    let using_oauth = credentials
        .as_ref()
        .is_some_and(|credentials| credentials.access_token == bearer_key);
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
