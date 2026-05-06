use crate::openhuman::config::Config;
use crate::openhuman::local_ai::model_ids;
use crate::openhuman::local_ai::ollama_api::{
    ns_to_tps, ollama_base_url, OllamaGenerateOptions, OllamaGenerateRequest,
};
use crate::openhuman::local_ai::parse::sanitize_inline_completion;

use super::LocalAiService;

impl LocalAiService {
    pub async fn summarize(
        &self,
        config: &Config,
        text: &str,
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        if !config.local_ai.runtime_enabled {
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
        if !config.local_ai.runtime_enabled {
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

    pub async fn inline_complete(
        &self,
        config: &Config,
        context: &str,
        style_preset: &str,
        style_instructions: Option<&str>,
        style_examples: &[String],
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        self.inline_complete_internal(
            config,
            context,
            style_preset,
            style_instructions,
            style_examples,
            max_tokens,
            /* gated = */ true,
        )
        .await
    }

    /// Latency-sensitive sibling of [`Self::inline_complete`] that
    /// **bypasses the scheduler gate's LLM permit**.
    ///
    /// Per-keystroke autocomplete must not block waiting for a
    /// long-running memory-tree backfill or a triage turn to release
    /// the global single slot. The user is at the keyboard; if the
    /// background pipeline is busy we'd rather race the autocomplete
    /// turn against it than show stale or empty completions for the
    /// duration of the backfill.
    ///
    /// This is the only path inside [`LocalAiService`] that opts out of
    /// the gate. Every other entry point (`inference`, `prompt`,
    /// `summarize`, `inline_complete`, `vision_prompt`, `embed`)
    /// acquires before talking to Ollama.
    pub async fn inline_complete_interactive(
        &self,
        config: &Config,
        context: &str,
        style_preset: &str,
        style_instructions: Option<&str>,
        style_examples: &[String],
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        log::trace!("[local_ai] inline_complete_interactive bypasses scheduler_gate permit");
        self.inline_complete_internal(
            config,
            context,
            style_preset,
            style_instructions,
            style_examples,
            max_tokens,
            /* gated = */ false,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn inline_complete_internal(
        &self,
        config: &Config,
        context: &str,
        style_preset: &str,
        style_instructions: Option<&str>,
        style_examples: &[String],
        max_tokens: Option<u32>,
        gated: bool,
    ) -> Result<String, String> {
        if !config.local_ai.runtime_enabled {
            return Ok(String::new());
        }

        let mut prompt = String::from(
            "You are an inline autocomplete engine.\n\
     Predict the most likely continuation of the user's partial text.\n\
     Return only the exact continuation suffix.\n\
     Do not repeat or rewrite any part of the user's existing text.\n\
     Do not include any prefix labels like 'suffix:' or 'completion:'.\n\
     Return exactly one line with plain text and no quotes.\n\
     Do not add leading or trailing whitespace.\n\
     Do not add tabs or newlines.\n\
     Do not add non-breaking spaces or zero-width characters.\n\
     Preserve natural spacing inside the continuation only when required between words.\n\
     If the user is in the middle of a word, continue that word directly with no space.\n\
     If the continuation is uncertain, return an empty string.\n",
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
        let escaped_context = context.replace("</USER_TEXT>", "<\\/USER_TEXT>");
        prompt.push_str("\nUser text (verbatim):\n<USER_TEXT>\n");
        prompt.push_str(&escaped_context);
        prompt.push_str("\n</USER_TEXT>");

        let raw = self
            .inference_with_temperature_allow_empty(
                config,
                "You are a low-latency inline text completion assistant.",
                &prompt,
                max_tokens.or(Some(24)),
                true,
                0.05,
                gated,
            )
            .await?;
        Ok(sanitize_inline_completion(&raw, context))
    }

    /// Multi-turn chat completion via Ollama /api/chat.
    /// Messages are `[{role: "user"|"assistant"|"system", content: "..."}]`.
    /// Returns the assistant reply string.
    pub(crate) async fn chat_with_history(
        &self,
        config: &Config,
        messages: Vec<crate::openhuman::local_ai::ollama_api::OllamaChatMessage>,
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        if !config.local_ai.runtime_enabled {
            return Err("local ai is disabled".to_string());
        }

        if !matches!(self.status.lock().state.as_str(), "ready") {
            self.bootstrap(config).await;
        }

        if messages.is_empty() {
            return Err("messages must not be empty".to_string());
        }

        // Multi-turn local chat is background LLM-bound work — gate it.
        let _gate_permit = crate::openhuman::scheduler_gate::wait_for_capacity().await;

        tracing::debug!(
            message_count = messages.len(),
            model = %crate::openhuman::local_ai::model_ids::effective_chat_model_id(config),
            "[local_ai:chat] sending to ollama /api/chat"
        );

        let started = std::time::Instant::now();

        let body = crate::openhuman::local_ai::ollama_api::OllamaChatRequest {
            model: crate::openhuman::local_ai::model_ids::effective_chat_model_id(config),
            messages,
            stream: false,
            options: Some(
                crate::openhuman::local_ai::ollama_api::OllamaGenerateOptions {
                    temperature: Some(config.default_temperature as f32),
                    top_k: Some(40),
                    top_p: Some(0.9),
                    num_predict: max_tokens.map(|v| v as i32),
                },
            ),
        };

        let response = self
            .http
            .post(format!("{}/api/chat", ollama_base_url()))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("ollama chat request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let detail = body.trim();
            return Err(format!(
                "ollama chat failed with status {}{}",
                status,
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            ));
        }

        let payload: crate::openhuman::local_ai::ollama_api::OllamaChatResponse = response
            .json()
            .await
            .map_err(|e| format!("ollama chat response parse failed: {e}"))?;

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

        tracing::debug!(
            elapsed_ms,
            reply_len = payload.message.content.len(),
            "[local_ai:chat] ollama /api/chat done"
        );

        let reply = payload.message.content.trim().to_string();
        if reply.is_empty() {
            Err("ollama returned empty reply".to_string())
        } else {
            Ok(reply)
        }
    }

    pub(crate) async fn inference(
        &self,
        config: &Config,
        system: &str,
        prompt: &str,
        max_tokens: Option<u32>,
        no_think: bool,
    ) -> Result<String, String> {
        self.inference_with_temperature(config, system, prompt, max_tokens, no_think, 0.2)
            .await
    }

    /// Latency-sensitive sibling of [`Self::inference`] that **bypasses
    /// the scheduler gate's LLM permit**.
    ///
    /// Used by user-arrival paths where the user is staring at the
    /// output (push-to-talk dictation cleanup, in particular). If we
    /// queue these behind a long-running memory backfill, the user
    /// experiences a frozen UI; better to race the call against
    /// background work and accept the contention than to silently
    /// degrade interactivity.
    ///
    /// Sibling to [`Self::inline_complete_interactive`] for autocomplete.
    /// Every other entry point (`inference`, `prompt`, `summarize`,
    /// `inline_complete`, `vision_prompt`, `embed`, `chat_with_history`)
    /// remains gated.
    pub(crate) async fn inference_interactive(
        &self,
        config: &Config,
        system: &str,
        prompt: &str,
        max_tokens: Option<u32>,
        no_think: bool,
    ) -> Result<String, String> {
        log::trace!("[local_ai] inference_interactive bypasses scheduler_gate permit");
        self.inference_with_temperature_internal(
            config, system, prompt, max_tokens, no_think, 0.2, /* allow_empty = */ false,
            /* gated = */ false,
        )
        .await
    }

    pub(crate) async fn inference_with_temperature(
        &self,
        config: &Config,
        system: &str,
        prompt: &str,
        max_tokens: Option<u32>,
        no_think: bool,
        temperature: f32,
    ) -> Result<String, String> {
        self.inference_with_temperature_internal(
            config,
            system,
            prompt,
            max_tokens,
            no_think,
            temperature,
            /* allow_empty = */ false,
            /* gated = */ true,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn inference_with_temperature_allow_empty(
        &self,
        config: &Config,
        system: &str,
        prompt: &str,
        max_tokens: Option<u32>,
        no_think: bool,
        temperature: f32,
        gated: bool,
    ) -> Result<String, String> {
        self.inference_with_temperature_internal(
            config,
            system,
            prompt,
            max_tokens,
            no_think,
            temperature,
            /* allow_empty = */ true,
            gated,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn inference_with_temperature_internal(
        &self,
        config: &Config,
        system: &str,
        prompt: &str,
        max_tokens: Option<u32>,
        no_think: bool,
        temperature: f32,
        allow_empty: bool,
        gated: bool,
    ) -> Result<String, String> {
        if !matches!(self.status.lock().state.as_str(), "ready") {
            self.bootstrap(config).await;
        }

        // Cooperative throttle + global single-slot acquisition for
        // background LLM-bound work. Drop happens at end of scope so
        // post-processing (status writes, logging) does NOT hold the
        // permit any longer than necessary. Interactive autocomplete
        // skips this via `gated = false` from
        // `inline_complete_interactive`.
        let _gate_permit = if gated {
            crate::openhuman::scheduler_gate::wait_for_capacity().await
        } else {
            None
        };

        let started = std::time::Instant::now();
        let model_id = model_ids::effective_chat_model_id(config);

        // When `no_think` is set, append the instruction to the system
        // prompt so the model treats it as a directive rather than content
        // it might parrot back.
        let effective_system = if no_think {
            tracing::debug!(
                no_think = true,
                max_tokens = ?max_tokens,
                allow_empty = allow_empty,
                model = %model_id,
                "[local_ai:infer] selected no_think prompt branch"
            );
            format!("{system}\n\nRespond with only the final answer. No reasoning, no preamble.")
        } else {
            tracing::debug!(
                no_think = false,
                max_tokens = ?max_tokens,
                allow_empty = allow_empty,
                model = %model_id,
                "[local_ai:infer] selected standard prompt branch"
            );
            system.to_string()
        };

        let body = OllamaGenerateRequest {
            model: model_id,
            prompt: prompt.to_string(),
            system: Some(effective_system),
            images: None,
            stream: false,
            options: Some(OllamaGenerateOptions {
                temperature: Some(temperature),
                top_k: Some(40),
                top_p: Some(0.9),
                num_predict: max_tokens.map(|v| v as i32),
            }),
        };

        let response = self
            .http
            .post(format!("{}/api/generate", ollama_base_url()))
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
            if allow_empty {
                Ok(String::new())
            } else {
                Err("ollama returned empty content".to_string())
            }
        } else {
            Ok(payload.response)
        }
    }
}

#[cfg(test)]
#[path = "public_infer_tests.rs"]
mod tests;
