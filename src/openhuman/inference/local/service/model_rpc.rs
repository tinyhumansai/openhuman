//! Thin local-model RPC boundary backed by TinyAgents.
//!
//! Local runtimes execute out of process. OpenHuman only resolves their HTTP
//! endpoint and adapts the legacy local-AI call shape to TinyAgents' provider-
//! neutral `ChatModel` interface.

use crate::openhuman::config::Config;
use crate::openhuman::inference::local::lm_studio::lm_studio_base_url;
use crate::openhuman::inference::local::ollama::{
    ollama_base_url_from_config, redact_ollama_base_url,
};
use crate::openhuman::inference::local::provider::{provider_from_config, LocalAiProvider};
use tinyagents::harness::message::Message;
use tinyagents::harness::model::{ChatModel, ModelRequest};
use tinyagents::harness::providers::openai::OpenAiModel;

pub(super) struct ModelRpcOutcome {
    pub reply: String,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub prompt_toks_per_sec: Option<f32>,
    pub gen_toks_per_sec: Option<f32>,
}

fn throughput(raw: Option<&serde_json::Value>, count: &str, duration: &str) -> Option<f32> {
    let raw = raw?;
    let count = raw.get(count)?.as_u64()?;
    let duration = raw.get(duration)?.as_u64()?;
    crate::openhuman::inference::local::ollama::ns_to_tps(count as f32, duration)
}

fn local_model(config: &Config, model_id: &str) -> Result<OpenAiModel, String> {
    let provider = provider_from_config(config);
    let model = match provider {
        LocalAiProvider::LmStudio => {
            let base = lm_studio_base_url(config);
            tracing::debug!(
                provider = provider.as_str(),
                endpoint = %redact_ollama_base_url(&base),
                has_api_key = config.local_ai.api_key.as_deref().is_some_and(|key| !key.trim().is_empty()),
                model = %model_id,
                "[local_ai:model_rpc] selecting LM Studio RPC model"
            );
            OpenAiModel::lm_studio(
                base,
                config.local_ai.api_key.as_deref().unwrap_or_default(),
                model_id,
            )
        }
        LocalAiProvider::Ollama => {
            let base = ollama_base_url_from_config(config);
            tracing::debug!(
                provider = provider.as_str(),
                endpoint = %redact_ollama_base_url(&base),
                model = %model_id,
                "[local_ai:model_rpc] selecting Ollama RPC model"
            );
            OpenAiModel::ollama_at(base, model_id)
        }
    };
    model.map_err(|error| {
        tracing::warn!(
            provider = provider.as_str(),
            error = %error,
            "[local_ai:model_rpc] model construction failed"
        );
        format!("invalid local model RPC configuration: {error}")
    })
}

pub(super) async fn invoke(
    config: &Config,
    client: reqwest::Client,
    messages: Vec<Message>,
    max_tokens: Option<u32>,
    temperature: f32,
    allow_empty: bool,
) -> Result<ModelRpcOutcome, String> {
    let model_id = crate::openhuman::inference::model_ids::effective_chat_model_id(config);
    let model = local_model(config, &model_id)?.with_client(client);
    let provider = provider_from_config(config);
    tracing::debug!(
        provider = provider.as_str(),
        model = %model_id,
        message_count = messages.len(),
        ?max_tokens,
        allow_empty,
        "[local_ai:model_rpc] invoking local model"
    );

    let mut request = ModelRequest::new(messages)
        .with_model(&model_id)
        .with_temperature(temperature as f64);
    if let Some(max_tokens) = max_tokens {
        request = request.with_max_tokens(max_tokens);
    }

    let response = model.invoke(&(), request).await.map_err(|error| {
        tracing::warn!(
            provider = provider.as_str(),
            model = %model_id,
            error = %error,
            "[local_ai:model_rpc] local model call failed"
        );
        if provider == LocalAiProvider::Ollama
            && error.to_string().contains("error sending request")
        {
            format!(
                "external Ollama endpoint is unavailable; ensure Ollama is already running: {error}"
            )
        } else {
            format!("local model RPC failed: {error}")
        }
    })?;
    let outcome = model_outcome(response, allow_empty)?;

    tracing::debug!(
        provider = provider.as_str(),
        model = %model_id,
        reply_len = outcome.reply.len(),
        prompt_tokens = ?outcome.prompt_tokens,
        completion_tokens = ?outcome.completion_tokens,
        "[local_ai:model_rpc] local model call completed"
    );
    Ok(outcome)
}

fn model_outcome(
    response: tinyagents::harness::model::ModelResponse,
    allow_empty: bool,
) -> Result<ModelRpcOutcome, String> {
    let mut reply = response.text();
    if reply.trim().is_empty() {
        reply = response
            .message
            .content
            .iter()
            .filter_map(|block| match block {
                tinyagents::harness::message::ContentBlock::Thinking { text, .. } => {
                    Some(text.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    let reply = reply.trim().to_owned();
    if reply.is_empty() && !allow_empty {
        return Err("local model RPC returned empty content".to_owned());
    }

    let prompt_tokens = response
        .usage
        .as_ref()
        .and_then(|usage| (usage.input_tokens > 0).then_some(usage.input_tokens));
    let completion_tokens = response
        .usage
        .as_ref()
        .and_then(|usage| (usage.output_tokens > 0).then_some(usage.output_tokens));
    let prompt_toks_per_sec = throughput(
        response.raw.as_ref(),
        "prompt_eval_count",
        "prompt_eval_duration",
    );
    let gen_toks_per_sec = throughput(response.raw.as_ref(), "eval_count", "eval_duration");

    Ok(ModelRpcOutcome {
        reply,
        prompt_tokens,
        completion_tokens,
        prompt_toks_per_sec,
        gen_toks_per_sec,
    })
}

#[cfg(test)]
mod tests {
    use super::{local_model, model_outcome, throughput};
    use crate::openhuman::config::Config;
    use tinyagents::harness::message::{AssistantMessage, ContentBlock};
    use tinyagents::harness::model::ModelResponse;
    use tinyagents::harness::usage::Usage;

    #[test]
    fn throughput_reads_ollama_timing_metadata() {
        let raw = serde_json::json!({
            "eval_count": 25,
            "eval_duration": 500_000_000_u64,
        });

        assert_eq!(
            throughput(Some(&raw), "eval_count", "eval_duration"),
            Some(50.0)
        );
        assert_eq!(
            throughput(Some(&raw), "prompt_eval_count", "prompt_eval_duration"),
            None
        );
    }

    #[test]
    fn local_model_selects_configured_provider() {
        let mut config = Config::default();
        config.local_ai.provider = "ollama".to_string();
        let ollama = local_model(&config, "qwen3").unwrap();
        assert_eq!(ollama.provider(), "ollama");

        config.local_ai.provider = "lm_studio".to_string();
        let lm_studio = local_model(&config, "local-model").unwrap();
        assert_eq!(lm_studio.provider(), "lm_studio");
    }

    #[test]
    fn model_outcome_enforces_empty_and_normalizes_usage() {
        let response = |text: &str, usage: Usage| ModelResponse {
            message: AssistantMessage {
                id: None,
                content: vec![ContentBlock::Text(text.to_string())],
                tool_calls: Vec::new(),
                usage: Some(usage),
            },
            usage: Some(usage),
            finish_reason: None,
            raw: None,
            resolved_model: None,
            continue_turn: None,
        };

        assert!(model_outcome(response(" ", Usage::default()), false).is_err());
        let empty = model_outcome(response(" ", Usage::default()), true).unwrap();
        assert_eq!(empty.reply, "");
        assert_eq!(empty.prompt_tokens, None);
        assert_eq!(empty.completion_tokens, None);

        let populated = model_outcome(
            response(
                "done",
                Usage {
                    input_tokens: 7,
                    output_tokens: 3,
                    ..Usage::default()
                },
            ),
            false,
        )
        .unwrap();
        assert_eq!(populated.prompt_tokens, Some(7));
        assert_eq!(populated.completion_tokens, Some(3));

        let reasoning_only = ModelResponse {
            message: AssistantMessage {
                id: None,
                content: vec![ContentBlock::Thinking {
                    text: "reasoning fallback".to_string(),
                    signature: None,
                }],
                tool_calls: Vec::new(),
                usage: None,
            },
            usage: None,
            finish_reason: None,
            raw: None,
            resolved_model: None,
            continue_turn: None,
        };
        assert_eq!(
            model_outcome(reasoning_only, false).unwrap().reply,
            "reasoning fallback"
        );
    }
}
