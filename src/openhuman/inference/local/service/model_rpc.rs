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
use tinyinference::message::Message;
use tinyinference::model::{ChatModel, ModelRequest};
use tinyinference::providers::openai::OpenAiModel;
use tinyinference::providers::{ProviderKind, ProviderSpec};

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
            // `OpenAiModel::lm_studio` no longer exists as a named preset; go
            // through `from_spec` with `ProviderKind::LmStudio` instead, which
            // routes to the same private `local_runtime` construction
            // (auth-style none, vision/native-tool-choice/json-object off,
            // context probing enabled) that the old preset used.
            OpenAiModel::from_spec(
                ProviderSpec {
                    kind: ProviderKind::LmStudio,
                    provider: "lm_studio".to_string(),
                    model: model_id.to_string(),
                    base_url: base,
                    api_key_env: None,
                    requires_api_key: false,
                },
                config.local_ai.api_key.as_deref().unwrap_or_default(),
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
    messages: Vec<Message>,
    max_tokens: Option<u32>,
    temperature: f32,
    allow_empty: bool,
) -> Result<ModelRpcOutcome, String> {
    let model_id = crate::openhuman::inference::model_ids::effective_chat_model_id(config);
    // `OpenAiModel` no longer exposes `with_client`/injects an external
    // `reqwest::Client` — each model now builds and owns its own client
    // internally (`OpenAiModel::new`). This host no longer shares its app-wide
    // HTTP client (connection pooling, proxy config) with local-inference
    // calls as a result; there is no replacement hook upstream.
    let model = local_model(config, &model_id)?;
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
    response: tinyinference::model::ModelResponse,
    allow_empty: bool,
) -> Result<ModelRpcOutcome, String> {
    let mut reply = response.text();
    if reply.trim().is_empty() {
        reply = response
            .message
            .content
            .iter()
            .filter_map(|block| match block {
                tinyinference::message::ContentBlock::Thinking { text, .. } => Some(text.as_str()),
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
#[path = "model_rpc_tests.rs"]
mod tests;
