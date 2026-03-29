use crate::openhuman::config::Config;
use crate::openhuman::local_ai::model_ids;
use crate::openhuman::local_ai::ollama_api::{
    ns_to_tps, OllamaGenerateOptions, OllamaGenerateRequest, OLLAMA_BASE_URL,
};
use crate::openhuman::local_ai::parse::{parse_suggestions, sanitize_inline_completion};
use crate::openhuman::local_ai::types::Suggestion;

use super::LocalAiService;

impl LocalAiService {
    pub async fn summarize(
        &self,
        config: &Config,
        text: &str,
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        if !config.local_ai.enabled {
            return Err("local ai is disabled".to_string());
        }
        let system = "You summarize internal assistant context. Keep concise bullet points.";
        let prompt = format!(
            "Summarize this text in concise bullet points. Preserve decisions and commitments.\\n\\n{}",
            text
        );
        self.inference(config, system, &prompt, max_tokens.or(Some(128)), true)
            .await
    }

    pub async fn prompt(
        &self,
        config: &Config,
        prompt: &str,
        max_tokens: Option<u32>,
        no_think: bool,
    ) -> Result<String, String> {
        if !config.local_ai.enabled {
            return Err("local ai is disabled".to_string());
        }
        let system = if no_think {
            "You are a concise assistant. Return only the final answer. Do not include reasoning or chain-of-thought."
        } else {
            "You are a helpful assistant."
        };
        self.inference(config, system, prompt, max_tokens.or(Some(160)), no_think)
            .await
    }

    pub async fn suggest_questions(
        &self,
        config: &Config,
        context: &str,
    ) -> Result<Vec<Suggestion>, String> {
        if !config.local_ai.enabled {
            return Ok(Vec::new());
        }
        let system = "You create short suggested user prompts.";
        let prompt = format!(
            "Given this conversation context, produce up to {} short suggested next user prompts. Return one prompt per line with no numbering.\\n\\n{}",
            config.local_ai.max_suggestions.max(1),
            context
        );
        let raw = self
            .inference(config, system, &prompt, Some(96), true)
            .await?;
        Ok(parse_suggestions(
            &raw,
            config.local_ai.max_suggestions.max(1),
        ))
    }

    pub async fn inline_complete(
        &self,
        config: &Config,
        context: &str,
        style_preset: &str,
        style_instructions: Option<&str>,
        style_examples: &[String],
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        if !config.local_ai.enabled {
            return Ok(String::new());
        }

        let mut prompt = String::from(
            "Complete the user's text with the most likely next words.\n\
             Return only the continuation suffix, no explanations.\n\
             Do not repeat text already written by the user.\n\
             Keep it natural and concise.\n\n",
        );
        prompt.push_str(&format!("Style preset: {}\n", style_preset.trim()));
        if let Some(instructions) = style_instructions {
            if !instructions.trim().is_empty() {
                prompt.push_str(&format!("Style instructions: {}\n", instructions.trim()));
            }
        }
        if !style_examples.is_empty() {
            prompt.push_str("Style examples:\n");
            for example in style_examples.iter().take(8) {
                let trimmed = example.trim();
                if !trimmed.is_empty() {
                    prompt.push_str("- ");
                    prompt.push_str(trimmed);
                    prompt.push('\n');
                }
            }
        }
        prompt.push_str("\nUser text:\n");
        prompt.push_str(context.trim());

        let raw = self
            .inference(
                config,
                "You are a low-latency inline text completion assistant.",
                &prompt,
                max_tokens.or(Some(36)),
                true,
            )
            .await?;
        Ok(sanitize_inline_completion(&raw))
    }

    pub(super) async fn inference(
        &self,
        config: &Config,
        system: &str,
        prompt: &str,
        max_tokens: Option<u32>,
        no_think: bool,
    ) -> Result<String, String> {
        if !matches!(self.status.lock().state.as_str(), "ready") {
            self.bootstrap(config).await;
        }

        let started = std::time::Instant::now();
        let mut combined_prompt = String::new();
        if no_think {
            combined_prompt.push_str("Respond with only the final answer. No reasoning.\\n\\n");
        }
        combined_prompt.push_str(prompt);

        let body = OllamaGenerateRequest {
            model: model_ids::effective_chat_model_id(config),
            prompt: combined_prompt,
            system: Some(system.to_string()),
            images: None,
            stream: false,
            options: Some(OllamaGenerateOptions {
                temperature: Some(0.2),
                top_k: Some(40),
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
            .map_err(|e| format!("ollama request failed: {e}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let detail = body.trim();
            return Err(format!(
                "ollama request failed with status {}{}",
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
            .map_err(|e| format!("ollama response parse failed: {e}"))?;

        let elapsed_ms = started.elapsed().as_millis() as u64;
        let prompt_tps = payload
            .prompt_eval_count
            .zip(payload.prompt_eval_duration)
            .and_then(|(count, dur_ns)| ns_to_tps(count as f32, dur_ns));
        let gen_tps = payload
            .eval_count
            .zip(payload.eval_duration)
            .and_then(|(count, dur_ns)| ns_to_tps(count as f32, dur_ns));

        {
            let mut status = self.status.lock();
            status.state = "ready".to_string();
            status.last_latency_ms = Some(elapsed_ms);
            status.prompt_toks_per_sec = prompt_tps;
            status.gen_toks_per_sec = gen_tps;
            status.warning = None;
        }

        if payload.response.trim().is_empty() {
            Err("ollama returned empty content".to_string())
        } else {
            Ok(payload.response)
        }
    }
}
