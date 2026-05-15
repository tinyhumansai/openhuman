use crate::openhuman::config::Config;
use crate::openhuman::local_ai::lm_studio_api::{
    apply_lm_studio_auth, lm_studio_base_url, LmStudioChatCompletionRequest,
    LmStudioChatCompletionResponse, LmStudioChatMessage, LmStudioModelsResponse,
};
use crate::openhuman::local_ai::model_ids;
use crate::openhuman::local_ai::ollama_api::OllamaModelTag;

use super::LocalAiService;

pub(in crate::openhuman::local_ai::service) struct LmStudioCompletionOutcome {
    pub reply: String,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
}

impl LocalAiService {
    pub(in crate::openhuman::local_ai::service) async fn ensure_lm_studio_available(
        &self,
        config: &Config,
    ) -> Result<(), String> {
        let models = self.list_lm_studio_models(config).await?;
        if models.is_empty() {
            return Err(format!(
                "LM Studio is reachable at {} but no models are loaded",
                lm_studio_base_url(config)
            ));
        }
        Ok(())
    }

    /// Returns `true` when the LM Studio server responds successfully to a
    /// `/v1/models` probe (regardless of whether any models are loaded).
    /// Reserved for future health-gating in the routing layer; currently only
    /// `ensure_lm_studio_available` is used at bootstrap.
    #[allow(dead_code)]
    pub(in crate::openhuman::local_ai::service) async fn lm_studio_healthy(
        &self,
        config: &Config,
    ) -> bool {
        self.list_lm_studio_models(config).await.is_ok()
    }

    pub(in crate::openhuman::local_ai::service) async fn list_lm_studio_models(
        &self,
        config: &Config,
    ) -> Result<Vec<OllamaModelTag>, String> {
        let base = lm_studio_base_url(config);
        let url = format!("{base}/models");
        tracing::debug!(
            target: "local_ai::lm_studio",
            %base,
            %url,
            "[local_ai:lm_studio] list_models: sending GET"
        );

        let request = self
            .http
            .get(&url)
            .timeout(std::time::Duration::from_secs(5));
        let response = apply_lm_studio_auth(request, config)
            .send()
            .await
            .map_err(|e| format!("lm studio models request failed: {e}"))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let detail = body.trim();
            return Err(format!(
                "lm studio models failed with status {}{}",
                status,
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            ));
        }

        let body = response
            .text()
            .await
            .map_err(|e| format!("lm studio models body read failed: {e}"))?;
        let payload: LmStudioModelsResponse = serde_json::from_str(&body)
            .map_err(|e| format!("lm studio models parse failed: {e}"))?;

        Ok(payload
            .data
            .into_iter()
            .map(|model| OllamaModelTag {
                name: model.id,
                size: None,
                modified_at: None,
            })
            .collect())
    }

    pub(in crate::openhuman::local_ai::service) async fn has_lm_studio_model(
        &self,
        config: &Config,
        model: &str,
    ) -> Result<bool, String> {
        let target = model.trim().to_ascii_lowercase();
        Ok(self
            .list_lm_studio_models(config)
            .await?
            .into_iter()
            .any(|m| m.name.to_ascii_lowercase() == target))
    }

    pub(in crate::openhuman::local_ai::service) async fn lm_studio_chat_completion(
        &self,
        config: &Config,
        messages: Vec<LmStudioChatMessage>,
        max_tokens: Option<u32>,
        temperature: f32,
        allow_empty: bool,
    ) -> Result<LmStudioCompletionOutcome, String> {
        let base = lm_studio_base_url(config);
        let url = format!("{base}/chat/completions");
        let model = model_ids::effective_chat_model_id(config);

        tracing::debug!(
            target: "local_ai::lm_studio",
            %url,
            %model,
            message_count = messages.len(),
            max_tokens = ?max_tokens,
            "[local_ai:lm_studio] chat completion: sending POST"
        );

        let body = LmStudioChatCompletionRequest {
            model,
            messages,
            stream: false,
            temperature: Some(temperature),
            max_tokens,
        };

        let request = self.http.post(&url).json(&body);
        let response = apply_lm_studio_auth(request, config)
            .send()
            .await
            .map_err(|e| format!("lm studio chat request failed: {e}"))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let detail = body.trim();
            return Err(format!(
                "lm studio chat failed with status {}{}",
                status,
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            ));
        }

        let payload: LmStudioChatCompletionResponse = response
            .json()
            .await
            .map_err(|e| format!("lm studio chat response parse failed: {e}"))?;

        let reply = payload
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_deref())
            .unwrap_or_default()
            .trim()
            .to_string();

        if reply.is_empty() && !allow_empty {
            return Err("lm studio returned empty content".to_string());
        }

        Ok(LmStudioCompletionOutcome {
            reply,
            prompt_tokens: payload.usage.as_ref().and_then(|usage| usage.prompt_tokens),
            completion_tokens: payload
                .usage
                .as_ref()
                .and_then(|usage| usage.completion_tokens),
        })
    }
}
