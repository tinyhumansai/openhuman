use crate::openhuman::config::Config;
use crate::openhuman::multimodal;
use crate::openhuman::local_ai::model_ids;
use crate::openhuman::local_ai::ollama_api::{
    OllamaEmbedRequest, OllamaEmbedResponse, OllamaGenerateOptions, OllamaGenerateRequest,
    OLLAMA_BASE_URL,
};
use crate::openhuman::local_ai::types::LocalAiEmbeddingResult;

use super::LocalAiService;

impl LocalAiService {
    pub async fn vision_prompt(
        &self,
        config: &Config,
        prompt: &str,
        image_refs: &[String],
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        if !config.local_ai.enabled {
            return Err("local ai is disabled".to_string());
        }
        if image_refs.is_empty() {
            return Err("vision prompt requires at least one image reference".to_string());
        }
        self.bootstrap(config).await;
        let vision_model = model_ids::effective_vision_model_id(config);
        self.ensure_ollama_model_available(&vision_model, "vision")
            .await?;

        let images: Vec<String> = image_refs
            .iter()
            .filter_map(|reference| multimodal::extract_ollama_image_payload(reference))
            .collect();
        if images.is_empty() {
            return Err("no valid image payloads were provided".to_string());
        }

        let body = OllamaGenerateRequest {
            model: vision_model,
            prompt: prompt.trim().to_string(),
            system: Some("You are a vision model. Answer directly and concisely.".to_string()),
            images: Some(images),
            stream: false,
            options: Some(OllamaGenerateOptions {
                temperature: Some(0.2),
                top_k: Some(30),
                top_p: Some(0.9),
                num_predict: max_tokens.map(|v| v as i32),
            }),
        };

        let response = self
            .http
            .post(format!("{OLLAMA_BASE_URL}/api/generate"))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("ollama vision request failed: {e}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let detail = body.trim();
            return Err(format!(
                "ollama vision request failed with status {}{}",
                status,
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            ));
        }

        let payload: crate::openhuman::local_ai::ollama_api::OllamaGenerateResponse = response
            .json()
            .await
            .map_err(|e| format!("ollama vision response parse failed: {e}"))?;
        if payload.response.trim().is_empty() {
            return Err("ollama vision returned empty content".to_string());
        }

        self.status.lock().vision_state = "ready".to_string();
        Ok(payload.response)
    }

    pub async fn embed(
        &self,
        config: &Config,
        inputs: &[String],
    ) -> Result<LocalAiEmbeddingResult, String> {
        if !config.local_ai.enabled {
            return Err("local ai is disabled".to_string());
        }
        let items: Vec<String> = inputs
            .iter()
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect();
        if items.is_empty() {
            return Err("embed requires at least one non-empty input".to_string());
        }
        self.bootstrap(config).await;
        let embedding_model = model_ids::effective_embedding_model_id(config);
        self.ensure_ollama_model_available(&embedding_model, "embedding")
            .await?;

        let response = self
            .http
            .post(format!("{OLLAMA_BASE_URL}/api/embed"))
            .json(&OllamaEmbedRequest {
                model: embedding_model.clone(),
                input: items.clone(),
            })
            .send()
            .await
            .map_err(|e| format!("ollama embed request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let detail = body.trim();
            return Err(format!(
                "ollama embed request failed with status {}{}",
                status,
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            ));
        }

        let payload: OllamaEmbedResponse = response
            .json()
            .await
            .map_err(|e| format!("ollama embed parse failed: {e}"))?;
        if payload.embeddings.is_empty() {
            return Err("ollama embed returned no embeddings".to_string());
        }

        let dims = payload.embeddings.first().map(|v| v.len()).unwrap_or(0);
        self.status.lock().embedding_state = "ready".to_string();
        Ok(LocalAiEmbeddingResult {
            model_id: embedding_model,
            dimensions: dims,
            vectors: payload.embeddings,
        })
    }
}
