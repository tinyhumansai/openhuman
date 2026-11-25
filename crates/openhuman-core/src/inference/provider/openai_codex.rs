use crate::config::Config;
use tinyinference_llm::providers::openai::codex::{
    OpenAiCodexRouting, OPENAI_CODEX_BACKEND_BASE_URL,
};

pub(crate) async fn resolve_openai_codex_routing(
    config: &Config,
    slug: &str,
    endpoint: &str,
    bearer_key: &str,
    bearer_is_oauth: bool,
) -> Result<OpenAiCodexRouting, String> {
    if slug != "openai" {
        return Ok(OpenAiCodexRouting::standard(endpoint));
    }

    let credentials = match crate::credentials::openai_oauth::lookup_openai_oauth_credentials(
        config,
    )
    .await
    {
        Ok(credentials) => credentials,
        Err(err) if !bearer_key.trim().is_empty() => {
            log::warn!(
                    "[providers][openai-codex] oauth metadata unavailable; continuing with standard bearer key: {err}"
                );
            None
        }
        Err(err) => return Err(format!("[chat-factory] openai oauth lookup failed: {err}")),
    };

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

/// Resolve Codex routing from the OAuth metadata that belongs to the exact
/// read-only credential selected for inspection. A plain API-key bearer must
/// not consult the separate saved OAuth profile at all.
pub(crate) fn resolve_openai_codex_routing_for_inspection(
    slug: &str,
    endpoint: &str,
    bearer_is_oauth: bool,
    account_id: Option<String>,
) -> OpenAiCodexRouting {
    if slug == "openai" && bearer_is_oauth {
        OpenAiCodexRouting {
            endpoint: OPENAI_CODEX_BACKEND_BASE_URL.to_string(),
            using_oauth: true,
            account_id: account_id
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        }
    } else {
        OpenAiCodexRouting::standard(endpoint)
    }
}
