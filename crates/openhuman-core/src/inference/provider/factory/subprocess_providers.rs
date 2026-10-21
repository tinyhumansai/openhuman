//! Subprocess-backed providers: the Claude Agent SDK and the Claude Code CLI.

use super::*;
use crate::inference::provider::factory::access_gates::verify_provider_session;

/// Build the Claude Agent SDK subprocess directly as a crate model. This is a
/// prompt-guided model: TinyAgents owns its text-tool protocol, while the
/// provider owns only subprocess transport and NDJSON decoding.
pub(super) fn try_create_claude_agent_sdk_chat_model(
    role: &str,
    config: &Config,
    egress: &EgressLedger,
) -> OptionalChatModelResult {
    let resolved = provider_for_role(role, config);
    try_create_claude_agent_sdk_chat_model_from_string(role, &resolved, config, egress)
}

pub(super) fn try_create_claude_agent_sdk_chat_model_from_string(
    role: &str,
    provider: &str,
    config: &Config,
    egress: &EgressLedger,
) -> OptionalChatModelResult {
    let model = match prepare_claude_agent_sdk_chat_model(role, provider, config)? {
        Ok(model) => model,
        Err(error) => return Some(Err(error)),
    };
    emit_inference_egress(role, &format!("{CLAUDE_AGENT_SDK_PREFIX}{model}"), egress);
    let chat: Arc<dyn ChatModel<()>> = Arc::new(ClaudeAgentSdkProvider::for_model(
        (&config.claude_agent_sdk).into(),
        model.clone(),
    ));
    Some(Ok((chat, model)))
}

/// Creates a bounded one-shot CLI model only when the explicit environment
/// credential can be frozen and passed to the inspection-only wrapper.
pub(super) fn try_create_claude_agent_sdk_chat_model_for_inspection(
    role: &str,
    provider: &str,
    config: &Config,
) -> InspectionChatModelResult {
    let model = claude_agent_sdk_model_from_string(provider, config)?;
    Some((|| {
        enforce_local_only_inference(role, provider)?;
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
        let auth =
            tinyagents_harness::providers::claude_agent_sdk::ClaudeAgentSdkInspectionAuth::new(
                api_key, auth_token, base_url,
            )?;
        let chat = ClaudeAgentSdkProvider::for_inspection(
            (&config.claude_agent_sdk).into(),
            model.clone(),
            auth,
        );
        Ok((
            Arc::new(chat) as Arc<dyn ChatModel<()>>,
            model.clone(),
            endpoint,
            credential,
        ))
    })())
}

pub(super) fn prepare_claude_agent_sdk_chat_model(
    role: &str,
    provider: &str,
    config: &Config,
) -> Option<anyhow::Result<String>> {
    let model = claude_agent_sdk_model_from_string(provider, config)?;
    if let Err(error) = enforce_local_only_inference(role, provider) {
        return Some(Err(error));
    }
    if let Err(error) = verify_provider_session(config, provider) {
        return Some(Err(error));
    }
    Some(Ok(model))
}

pub(super) fn claude_agent_sdk_model_from_string(
    provider: &str,
    config: &Config,
) -> Option<String> {
    let provider = provider.trim();
    let model = if provider == CLAUDE_AGENT_SDK_PROVIDER {
        config.claude_agent_sdk.default_model.clone()
    } else {
        let model = provider.strip_prefix(CLAUDE_AGENT_SDK_PREFIX)?;
        model.trim().to_string()
    };
    Some(model)
}

pub(super) fn try_create_claude_code_chat_model(
    role: &str,
    config: &Config,
    model_override: Option<&str>,
    egress: &EgressLedger,
) -> OptionalChatModelResult {
    let resolved = provider_for_role(role, config);
    try_create_claude_code_chat_model_from_string(role, &resolved, config, model_override, egress)
}

pub(super) fn try_create_claude_code_chat_model_from_string(
    role: &str,
    provider: &str,
    config: &Config,
    model_override: Option<&str>,
    egress: &EgressLedger,
) -> OptionalChatModelResult {
    let provider = provider.trim();
    let model_with_temp =
        provider.strip_prefix(tinyagents_harness::providers::claude_code::PROVIDER_PREFIX)?;
    let (configured_model, temperature_override) = split_model_and_temperature(model_with_temp);
    if temperature_override.is_some() {
        log::warn!(
            "[providers][chat-factory] claude-code provider: per-model temperature override \
             is accepted but not wired through to the CLI — the @<temp> suffix is ignored"
        );
    }
    if configured_model.is_empty() {
        return Some(Err(anyhow::anyhow!(
            "[chat-factory] provider string '{}' for role '{}' has an empty model — \
             use 'claude-code:<model-id>'",
            provider,
            role
        )));
    }
    if let Err(error) = enforce_local_only_inference(role, provider) {
        return Some(Err(error));
    }
    if let Err(error) = verify_provider_session(config, provider) {
        return Some(Err(error));
    }
    let workspace = config
        .config_path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| config.workspace_dir.clone());
    let effective_model = model_override.unwrap_or(&configured_model).to_string();
    emit_inference_egress(
        role,
        &format!(
            "{}{effective_model}",
            tinyagents_harness::providers::claude_code::PROVIDER_PREFIX
        ),
        egress,
    );
    let chat = match tinyagents_harness::providers::claude_code::ClaudeCodeProvider::from_env(
        effective_model,
        workspace,
        config.action_dir.clone(),
    ) {
        Ok(model) => Arc::new(model.with_mcp_provider(Arc::new(OpenHumanMcpEndpoint)))
            as Arc<dyn ChatModel<()>>,
        Err(error) => return Some(Err(error)),
    };
    Some(Ok((chat, configured_model)))
}

/// Uses the same Claude CLI for a one-shot, tool-free inspection request. The
/// route is unavailable without an explicit API credential; the saved CLI
/// session is never read.
pub(super) fn try_create_claude_code_chat_model_for_inspection(
    role: &str,
    provider: &str,
    config: &Config,
) -> InspectionChatModelResult {
    let provider = provider.trim();
    let model_with_temp =
        provider.strip_prefix(tinyagents_harness::providers::claude_code::PROVIDER_PREFIX)?;
    let (configured_model, _) = split_model_and_temperature(model_with_temp);
    if configured_model.is_empty() {
        return Some(Err(anyhow::anyhow!("Claude CLI route has an empty model")));
    }
    let sdk_provider = format!("{CLAUDE_AGENT_SDK_PREFIX}{configured_model}");
    try_create_claude_agent_sdk_chat_model_for_inspection(role, &sdk_provider, config)
}

#[derive(Debug)]
struct OpenHumanMcpEndpoint;

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
