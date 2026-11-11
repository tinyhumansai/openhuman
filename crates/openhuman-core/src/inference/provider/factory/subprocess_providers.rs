//! OpenHuman host endpoint adapter and frozen Claude inspection route.

use super::*;
use tinyagents_harness::providers::claude_agent_sdk::ClaudeAgentSdkProvider;

/// Builds only an explicitly authenticated, bounded SDK model for doctor.
/// CLI-managed credentials are never consulted, so inspection cannot inherit
/// or mutate an interactive provider session.
pub(super) fn try_create_claude_agent_sdk_chat_model_for_inspection(
    role: &str,
    provider: &str,
    config: &Config,
) -> InspectionChatModelResult {
    let model = claude_agent_sdk_model_from_string(provider, config)?;
    Some((|| {
        enforce_local_only_inference(role, provider)?;
        if model.is_empty() {
            anyhow::bail!("Claude Agent SDK route has an empty model");
        }
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|value| !value.is_empty());
        let auth_token = std::env::var("ANTHROPIC_AUTH_TOKEN")
            .ok()
            .filter(|value| !value.is_empty());
        let credential = api_key
            .as_deref()
            .or(auth_token.as_deref())
            .ok_or_else(|| anyhow::anyhow!("explicit Claude API credential is unavailable"))?
            .to_string();
        let base_url = std::env::var("ANTHROPIC_BASE_URL")
            .ok()
            .filter(|value| !value.is_empty());
        let endpoint = base_url
            .as_deref()
            .unwrap_or("https://api.anthropic.com")
            .trim_end_matches('/')
            .to_string();
        let model_id = model.clone();
        let auth =
            tinyagents_harness::providers::claude_agent_sdk::ClaudeAgentSdkInspectionAuth::new(
                api_key, auth_token, base_url,
            )?;
        let model = ClaudeAgentSdkProvider::for_inspection(
            (&config.claude_agent_sdk).into(),
            model.clone(),
            auth,
        );
        Ok((
            Arc::new(model) as Arc<dyn ChatModel<()>>,
            model_id,
            endpoint,
            credential,
        ))
    })())
}

fn claude_agent_sdk_model_from_string(provider: &str, config: &Config) -> Option<String> {
    let provider = provider.trim();
    if provider == CLAUDE_AGENT_SDK_PROVIDER {
        Some(config.claude_agent_sdk.default_model.clone())
    } else {
        // The model as the runtime route reads it: an `@<temperature>`
        // suffix is not part of the model id.
        let model = provider.strip_prefix(CLAUDE_AGENT_SDK_PREFIX)?.trim();
        Some(split_model_and_temperature(model).0.trim().to_owned())
    }
}

/// Claude Code inspection uses the same frozen, tool-free SDK path; it never
/// reads the CLI's saved session.
pub(super) fn try_create_claude_code_chat_model_for_inspection(
    role: &str,
    provider: &str,
    config: &Config,
) -> InspectionChatModelResult {
    let provider = provider.trim();
    let model =
        provider.strip_prefix(tinyagents_harness::providers::claude_code::PROVIDER_PREFIX)?;
    let (model, _) = split_model_and_temperature(model);
    if model.is_empty() {
        return Some(Err(anyhow::anyhow!("Claude CLI route has an empty model")));
    }
    let sdk_provider = format!("{CLAUDE_AGENT_SDK_PREFIX}{model}");
    try_create_claude_agent_sdk_chat_model_for_inspection(role, &sdk_provider, config)
}

#[derive(Debug)]
pub(super) struct OpenHumanMcpEndpoint;

#[async_trait::async_trait]
impl tinyagents_harness::providers::claude_code::driver::McpEndpointProvider
    for OpenHumanMcpEndpoint
{
    async fn endpoint(
        &self,
    ) -> Result<tinyagents_harness::providers::claude_code::driver::McpEndpoint, String> {
        let endpoint = crate::mcp::server::ensure_local_http()
            .await
            .map_err(|error| error.to_string())?;
        Ok(
            tinyagents_harness::providers::claude_code::driver::McpEndpoint {
                addr: endpoint.addr,
                token: endpoint.token,
            },
        )
    }
}
