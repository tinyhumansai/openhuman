use crate::openhuman::inference::provider::traits::{
    ChatMessage, ChatRequest as ProviderChatRequest, ChatResponse as ProviderChatResponse,
    Provider, StreamChunk, StreamError, StreamOptions, StreamResult, ToolCall as ProviderToolCall,
};
use async_trait::async_trait;
use futures_util::{stream, StreamExt};

use super::compatible_dump::{dump_prompt_if_enabled, dump_response_if_enabled, reserve_dump_seq};
use super::compatible_parse::normalize_function_arguments;
use super::compatible_stream::sse_bytes_to_chunks;
use super::compatible_types::{
    ApiChatRequest, ApiChatResponse, Message, MessageContent, NativeChatRequest,
    OpenAiStreamOptions,
};
use super::{AuthStyle, OpenAiCompatibleProvider};

#[async_trait]
impl Provider for OpenAiCompatibleProvider {
    fn capabilities(&self) -> crate::openhuman::inference::provider::traits::ProviderCapabilities {
        crate::openhuman::inference::provider::traits::ProviderCapabilities {
            native_tool_calling: self.native_tool_calling,
            vision: self.vision,
        }
    }

    fn prompt_cache_capabilities(
        &self,
    ) -> crate::openhuman::inference::provider::traits::PromptCacheCapabilities {
        // Derive from the configured slug — conservative for unknown / custom
        // providers (#3939). The OpenHuman backend wraps this provider but
        // declares its own grouping-aware caps on `OpenHumanBackendProvider`.
        super::prompt_cache_for_compatible_slug(&self.name)
    }

    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let credential = self.credential_for_request()?;

        let mut messages = Vec::new();

        if self.merge_system_into_user {
            let content = match system_prompt {
                Some(sys) => format!("{sys}\n\n{message}"),
                None => message.to_string(),
            };
            messages.push(Message {
                role: "user".to_string(),
                content: MessageContent::from_chat_text(&content),
            });
        } else {
            if let Some(sys) = system_prompt {
                messages.push(Message {
                    role: "system".to_string(),
                    content: sys.into(),
                });
            }
            messages.push(Message {
                role: "user".to_string(),
                content: MessageContent::from_chat_text(message),
            });
        }

        let request = ApiChatRequest {
            model: model.to_string(),
            messages,
            temperature: self.effective_temperature(model, temperature),
            stream: Some(false),
            tools: None,
            tool_choice: None,
        };

        let url = self.chat_completions_url();

        let mut fallback_messages = Vec::new();
        if let Some(system_prompt) = system_prompt {
            fallback_messages.push(ChatMessage::system(system_prompt));
        }
        fallback_messages.push(ChatMessage::user(message));
        let fallback_messages = if self.merge_system_into_user {
            Self::flatten_system_messages(&fallback_messages)
        } else {
            fallback_messages
        };

        if self.responses_api_primary {
            return self
                .chat_via_responses(credential, &fallback_messages, model, None)
                .await;
        }

        let response = match self
            .apply_auth_header(self.http_client().post(&url).json(&request), credential)
            .send()
            .await
        {
            Ok(response) => response,
            Err(chat_error) => {
                if self.supports_responses_fallback {
                    let detail = super::super::format_error_chain(&chat_error);
                    return self
                        .chat_via_responses(credential, &fallback_messages, model, None)
                        .await
                        .map_err(|responses_err| {
                            let fb = super::super::format_anyhow_chain(&responses_err);
                            anyhow::anyhow!(
                                "{} chat completions transport error: {detail} (responses fallback failed: {fb})",
                                self.name
                            )
                        });
                }

                return Err(chat_error.into());
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let error = response.text().await?;
            let sanitized = super::super::sanitize_api_error(&error);

            if let Some(err) = self.completion_only_404_guard(status, &sanitized, model) {
                return Err(err);
            }

            if let Some(err) = self.not_chat_capable_guard(status, &sanitized, model) {
                return Err(err);
            }

            if status == reqwest::StatusCode::NOT_FOUND && self.supports_responses_fallback {
                return self
                    .chat_via_responses(credential, &fallback_messages, model, None)
                    .await
                    .map_err(|responses_err| {
                        let fb = super::super::format_anyhow_chain(&responses_err);
                        anyhow::anyhow!(
                            "{} API error ({status}): {sanitized} (chat completions unavailable; responses fallback failed: {fb})",
                            self.name
                        )
                    });
            }

            let status_str = status.as_u16().to_string();
            let message = self.enrich_404_message(
                format!("{} API error ({status}): {sanitized}", self.name),
                status,
            );
            if super::super::is_backend_auth_failure(self.name.as_str(), status) {
                super::super::publish_backend_session_expired(
                    "chat_completions",
                    self.name.as_str(),
                    status,
                    &message,
                );
            } else if super::super::is_budget_exhausted_http_400(status, &error) {
                super::super::log_budget_exhausted_http_400(
                    "chat_completions",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::super::is_custom_openai_upstream_bad_request_http_400(
                self.name.as_str(),
                status,
                &error,
            ) {
                super::super::log_custom_openai_upstream_bad_request_http_400(
                    "chat_completions",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::super::is_provider_access_policy_denied_http_403(status, &error) {
                super::super::log_provider_access_policy_denied_http_403(
                    "chat_completions",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::super::is_provider_config_rejection_http(
                status,
                self.name.as_str(),
                &error,
            ) {
                super::super::log_provider_config_rejection(
                    "chat_completions",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::super::is_byo_provider_auth_failure_http(
                self.name.as_str(),
                status,
                &error,
            ) {
                super::super::log_byo_provider_auth_failure(
                    "chat_completions",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::super::is_provider_insufficient_credits_402(status, &error) {
                // Insufficient-credits 402: the user's own BYO provider account
                // is out of balance — a flat billing fact, not a reservation-
                // window error, so there is NO local max_tokens lever to apply.
                // Demote to info instead of paging on every retry; this is the
                // complete classification for a genuinely-unpreventable
                // BYO-balance condition (TAURI-RUST-4QF — DeepSeek "Insufficient
                // Balance"; same class as the native_chat arm added for -C62).
                super::super::log_provider_insufficient_credits_402(
                    "chat_completions",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::super::should_report_provider_http_failure(status) {
                crate::core::observability::report_error(
                    message.as_str(),
                    "llm_provider",
                    "chat_completions",
                    &[
                        ("provider", self.name.as_str()),
                        ("model", model),
                        ("status", status_str.as_str()),
                        ("failure", "non_2xx"),
                    ],
                );
            }
            anyhow::bail!(message);
        }

        let body = response.text().await?;
        let chat_response = super::compatible_parse::parse_chat_response_body(&self.name, &body)?;

        chat_response
            .choices
            .into_iter()
            .next()
            .map(|c| {
                if c.message.tool_calls.is_some()
                    && c.message.tool_calls.as_ref().is_some_and(|t| !t.is_empty())
                {
                    serde_json::to_string(&c.message)
                        .unwrap_or_else(|_| c.message.effective_content())
                } else {
                    c.message.effective_content()
                }
            })
            .ok_or_else(|| anyhow::anyhow!("No response from {}", self.name))
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let credential = self.credential_for_request()?;

        let effective_messages = if self.merge_system_into_user {
            Self::flatten_system_messages(messages)
        } else {
            messages.to_vec()
        };
        let api_messages: Vec<Message> = effective_messages
            .iter()
            .map(|m| Message {
                role: m.role.clone(),
                content: MessageContent::from_chat_text(&m.content),
            })
            .collect();

        let request = ApiChatRequest {
            model: model.to_string(),
            messages: api_messages,
            temperature: self.effective_temperature(model, temperature),
            stream: Some(false),
            tools: None,
            tool_choice: None,
        };

        let url = self.chat_completions_url();
        if self.responses_api_primary {
            return self
                .chat_via_responses(credential, &effective_messages, model, None)
                .await;
        }

        let response = match self
            .apply_auth_header(self.http_client().post(&url).json(&request), credential)
            .send()
            .await
        {
            Ok(response) => response,
            Err(chat_error) => {
                if self.supports_responses_fallback {
                    let detail = super::super::format_error_chain(&chat_error);
                    return self
                        .chat_via_responses(credential, &effective_messages, model, None)
                        .await
                        .map_err(|responses_err| {
                            let fb = super::super::format_anyhow_chain(&responses_err);
                            anyhow::anyhow!(
                                "{} chat completions transport error: {detail} (responses fallback failed: {fb})",
                                self.name
                            )
                        });
                }

                return Err(chat_error.into());
            }
        };

        if !response.status().is_success() {
            let status = response.status();

            if status == reqwest::StatusCode::NOT_FOUND {
                let error = response.text().await?;
                let sanitized = super::super::sanitize_api_error(&error);

                if let Some(err) = self.completion_only_404_guard(status, &sanitized, model) {
                    return Err(err);
                }

                if self.supports_responses_fallback {
                    return self
                        .chat_via_responses(credential, &effective_messages, model, None)
                        .await
                        .map_err(|responses_err| {
                            let fb = super::super::format_anyhow_chain(&responses_err);
                            anyhow::anyhow!(
                                "{} API error ({status}): {sanitized} (chat completions unavailable; responses fallback failed: {fb})",
                                self.name
                            )
                        });
                }

                let enriched = self.enrich_404_message(
                    format!("{} API error ({status}): {sanitized}", self.name),
                    status,
                );
                return Err(anyhow::anyhow!("{enriched}"));
            }

            let err = super::super::api_error(&self.name, response).await;
            let err_str = err.to_string();
            if Self::is_not_chat_capable_model(status, &err_str) {
                return Err(anyhow::anyhow!(
                    self.not_chat_capable_model_message(model, &err_str)
                ));
            }
            let enriched = self.enrich_404_message(format!("{err:#}"), status);
            return Err(anyhow::anyhow!("{enriched}"));
        }

        let body = response.text().await?;
        let chat_response = super::compatible_parse::parse_chat_response_body(&self.name, &body)?;

        chat_response
            .choices
            .into_iter()
            .next()
            .map(|c| {
                if c.message.tool_calls.is_some()
                    && c.message.tool_calls.as_ref().is_some_and(|t| !t.is_empty())
                {
                    serde_json::to_string(&c.message)
                        .unwrap_or_else(|_| c.message.effective_content())
                } else {
                    c.message.effective_content()
                }
            })
            .ok_or_else(|| anyhow::anyhow!("No response from {}", self.name))
    }

    async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ProviderChatResponse> {
        let credential = self.credential_for_request()?;

        let effective_messages = if self.merge_system_into_user {
            Self::flatten_system_messages(messages)
        } else {
            messages.to_vec()
        };
        let api_messages: Vec<Message> = effective_messages
            .iter()
            .map(|m| Message {
                role: m.role.clone(),
                content: MessageContent::from_chat_text(&m.content),
            })
            .collect();

        let request = ApiChatRequest {
            model: model.to_string(),
            messages: api_messages,
            temperature: self.effective_temperature(model, temperature),
            stream: Some(false),
            tools: if tools.is_empty() {
                None
            } else {
                Some(tools.to_vec())
            },
            tool_choice: if tools.is_empty() {
                None
            } else {
                Some("auto".to_string())
            },
        };

        let url = self.chat_completions_url();
        let response = match self
            .apply_auth_header(self.http_client().post(&url).json(&request), credential)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(
                    "{} native tool call transport failed: {error}; falling back to history path",
                    self.name
                );
                let text = self.chat_with_history(messages, model, temperature).await?;
                return Ok(ProviderChatResponse {
                    text: Some(text),
                    tool_calls: vec![],
                    usage: None,
                    reasoning_content: None,
                });
            }
        };

        if !response.status().is_success() {
            return Err(super::super::api_error(&self.name, response).await);
        }

        let body = response.text().await?;
        let chat_response = super::compatible_parse::parse_chat_response_body(&self.name, &body)?;
        let usage = Self::extract_usage(&chat_response);
        let choice = chat_response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No response from {}", self.name))?;

        let text = choice.message.effective_content_optional();
        let reasoning_content = choice
            .message
            .reasoning_content
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        let tool_calls = choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .filter_map(|tc| {
                let function = tc.function?;
                let name = function.name?;
                let arguments = normalize_function_arguments(function.arguments);
                Some(ProviderToolCall {
                    // Treat an empty-string id like a missing one: some providers
                    // (DashScope/GMI) return `""` rather than omitting it, and an
                    // empty `tool_call_id` is rejected by the upstream tool-message
                    // ordering check on the next turn.
                    id: tc
                        .id
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                    name,
                    arguments,
                    // Non-streaming response: preserve Gemini's thought_signature
                    // so it round-trips on the next turn (TAURI-RUST-4PK).
                    extra_content: tc.extra_content,
                })
            })
            .collect::<Vec<_>>();

        tracing::debug!(
            has_reasoning_content = reasoning_content.is_some(),
            reasoning_content_chars = reasoning_content.as_ref().map_or(0, |r| r.chars().count()),
            tool_calls = tool_calls.len(),
            "[provider:chat] reasoning_content capture (non-streaming)"
        );

        Ok(ProviderChatResponse {
            text,
            tool_calls,
            usage,
            reasoning_content,
        })
    }

    async fn chat(
        &self,
        request: ProviderChatRequest<'_>,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ProviderChatResponse> {
        let credential = self.credential_for_request()?;

        let tools = Self::convert_tool_specs(request.tools);
        let effective_messages = if self.merge_system_into_user {
            Self::flatten_system_messages(request.messages)
        } else {
            request.messages.to_vec()
        };

        if self.responses_api_primary {
            let response_messages = if request.tools.is_some() {
                Self::with_prompt_guided_tool_instructions(request.messages, request.tools)
            } else {
                effective_messages.clone()
            };
            let text = self
                .chat_via_responses(credential, &response_messages, model, request.max_tokens)
                .await?;
            if let Some(tx) = request.stream {
                let _ = tx
                    .send(
                        crate::openhuman::inference::provider::ProviderDelta::TextDelta {
                            delta: text.clone(),
                        },
                    )
                    .await;
            }
            return Ok(ProviderChatResponse {
                text: Some(text),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            });
        }

        if let Some(tx) = request.stream {
            let native_request = NativeChatRequest {
                model: model.to_string(),
                messages: Self::convert_messages_for_native(&effective_messages),
                temperature: self.effective_temperature(model, temperature),
                stream: Some(true),
                tool_choice: tools.as_ref().map(|_| "auto".to_string()),
                tools: tools.clone(),
                thread_id: self.outbound_thread_id(),
                stream_options: Some(OpenAiStreamOptions {
                    include_usage: true,
                }),
                options: self.build_ollama_options(),
                // Omitted for endpoints whose OpenAI-compat surface 400s on the
                // field (Google Gemini shim — TAURI-RUST-4PJ); the reactive
                // retry below stays as defense-in-depth for unknown providers.
                frequency_penalty: self.effective_frequency_penalty(),
                max_tokens: request.max_tokens,
            };
            let stream_dump_seq = reserve_dump_seq();
            dump_prompt_if_enabled(&self.name, model, stream_dump_seq, &native_request);
            match self
                .stream_native_chat(credential, &native_request, tx, stream_dump_seq)
                .await
            {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    let err_str = err.to_string();
                    if tools.is_some() && Self::err_supports_no_tools_retry(&err_str) {
                        log::info!(
                            "[stream] {} model does not support tools — retrying streaming without tools",
                            self.name,
                        );
                        let retry_request = NativeChatRequest {
                            tools: None,
                            tool_choice: None,
                            ..native_request.clone()
                        };
                        match self
                            .stream_native_chat(credential, &retry_request, tx, stream_dump_seq)
                            .await
                        {
                            Ok(resp) => return Ok(resp),
                            Err(retry_err) => {
                                log::warn!(
                                    "[stream] {} retry without tools also failed, falling back to non-streaming: {}",
                                    self.name,
                                    retry_err
                                );
                            }
                        }
                    } else if Self::err_indicates_frequency_penalty_unsupported(&err_str) {
                        log::info!(
                            "[stream] {} rejected frequency_penalty — retrying streaming without it",
                            self.name,
                        );
                        let retry_request = NativeChatRequest {
                            frequency_penalty: None,
                            ..native_request.clone()
                        };
                        match self
                            .stream_native_chat(credential, &retry_request, tx, stream_dump_seq)
                            .await
                        {
                            Ok(resp) => return Ok(resp),
                            Err(retry_err) => {
                                log::warn!(
                                    "[stream] {} retry without frequency_penalty also failed, falling back to non-streaming: {}",
                                    self.name,
                                    retry_err
                                );
                            }
                        }
                    } else {
                        log::warn!(
                            "[stream] {} streaming chat failed, falling back to non-streaming: {}",
                            self.name,
                            err
                        );
                    }
                }
            }
        }

        let thread_id = self.outbound_thread_id();
        log::debug!(
            "[provider:{}] chat() outbound thread_id={} model={}",
            self.name,
            thread_id.as_deref().unwrap_or("<none>"),
            model
        );
        let native_request = NativeChatRequest {
            model: model.to_string(),
            messages: Self::convert_messages_for_native(&effective_messages),
            temperature: self.effective_temperature(model, temperature),
            stream: Some(false),
            tool_choice: tools.as_ref().map(|_| "auto".to_string()),
            tools,
            thread_id,
            stream_options: None,
            options: self.build_ollama_options(),
            // The buffered non-streaming path omits `frequency_penalty` for maximum
            // compatibility. The streaming path carries it and retries without on rejection.
            frequency_penalty: None,
            max_tokens: request.max_tokens,
        };
        let dump_seq = reserve_dump_seq();
        dump_prompt_if_enabled(&self.name, model, dump_seq, &native_request);

        let url = self.chat_completions_url();
        let response = match self
            .apply_auth_header(
                self.http_client().post(&url).json(&native_request),
                credential,
            )
            .send()
            .await
        {
            Ok(response) => response,
            Err(chat_error) => {
                if self.supports_responses_fallback {
                    let detail = super::super::format_error_chain(&chat_error);
                    return self
                        .chat_via_responses(
                            credential,
                            &effective_messages,
                            model,
                            request.max_tokens,
                        )
                        .await
                        .map(|text| ProviderChatResponse {
                            text: Some(text),
                            tool_calls: vec![],
                            usage: None,
                            reasoning_content: None,
                        })
                        .map_err(|responses_err| {
                            let fb = super::super::format_anyhow_chain(&responses_err);
                            anyhow::anyhow!(
                                "{} native chat transport error: {detail} (responses fallback failed: {fb})",
                                self.name
                            )
                        });
                }

                return Err(chat_error.into());
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let error = response.text().await?;
            let sanitized = super::super::sanitize_api_error(&error);

            if Self::is_native_tool_schema_unsupported(status, &sanitized) {
                let fallback_messages =
                    Self::with_prompt_guided_tool_instructions(request.messages, request.tools);
                let text = self
                    .chat_with_history(&fallback_messages, model, temperature)
                    .await?;
                return Ok(ProviderChatResponse {
                    text: Some(text),
                    tool_calls: vec![],
                    usage: None,
                    reasoning_content: None,
                });
            }

            if let Some(err) = self.completion_only_404_guard(status, &sanitized, model) {
                return Err(err);
            }

            if let Some(err) = self.not_chat_capable_guard(status, &sanitized, model) {
                return Err(err);
            }

            if status == reqwest::StatusCode::NOT_FOUND && self.supports_responses_fallback {
                return self
                    .chat_via_responses(credential, &effective_messages, model, request.max_tokens)
                    .await
                    .map(|text| ProviderChatResponse {
                        text: Some(text),
                        tool_calls: vec![],
                        usage: None,
                        reasoning_content: None,
                    })
                    .map_err(|responses_err| {
                        let fb = super::super::format_anyhow_chain(&responses_err);
                        anyhow::anyhow!(
                            "{} API error ({status}): {sanitized} (chat completions unavailable; responses fallback failed: {fb})",
                            self.name
                        )
                    });
            }

            let status_str = status.as_u16().to_string();
            let message = self.enrich_404_message(
                format!("{} API error ({status}): {sanitized}", self.name),
                status,
            );
            if super::super::is_budget_exhausted_http_400(status, &error) {
                super::super::log_budget_exhausted_http_400(
                    "native_chat",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::super::is_custom_openai_upstream_bad_request_http_400(
                self.name.as_str(),
                status,
                &error,
            ) {
                super::super::log_custom_openai_upstream_bad_request_http_400(
                    "native_chat",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::super::is_provider_access_policy_denied_http_403(status, &error) {
                super::super::log_provider_access_policy_denied_http_403(
                    "native_chat",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::super::is_provider_config_rejection_http(
                status,
                self.name.as_str(),
                &error,
            ) {
                super::super::log_provider_config_rejection(
                    "native_chat",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::super::is_byo_provider_auth_failure_http(
                self.name.as_str(),
                status,
                &error,
            ) {
                super::super::log_byo_provider_auth_failure(
                    "native_chat",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::super::is_provider_insufficient_credits_402(status, &error) {
                // Residual 402 after the request already caps max_tokens: the
                // user's own BYO provider balance is exhausted — no local lever,
                // so demote to info instead of paging on every retry
                // (TAURI-RUST-C62).
                super::super::log_provider_insufficient_credits_402(
                    "native_chat",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::super::should_report_provider_http_failure(status) {
                crate::core::observability::report_error(
                    message.as_str(),
                    "llm_provider",
                    "native_chat",
                    &[
                        ("provider", self.name.as_str()),
                        ("model", model),
                        ("status", status_str.as_str()),
                        ("failure", "non_2xx"),
                    ],
                );
            }
            anyhow::bail!(message);
        }

        let response_bytes = response.bytes().await?;
        dump_response_if_enabled(&self.name, model, dump_seq, &response_bytes);
        let native_response: ApiChatResponse = serde_json::from_slice(&response_bytes)
            .map_err(|err| anyhow::anyhow!("{} response parse error: {err}", self.name))?;
        Self::parse_native_response(native_response, &self.name)
    }

    fn supports_native_tools(&self) -> bool {
        self.native_tool_calling
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn stream_chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
        options: StreamOptions,
    ) -> stream::BoxStream<'static, StreamResult<StreamChunk>> {
        let credential = match self.credential_for_request() {
            Ok(value) => value.map(str::to_string),
            Err(err) => {
                return stream::once(async move { Err(StreamError::Provider(err.to_string())) })
                    .boxed();
            }
        };

        let mut messages = Vec::new();
        if let Some(sys) = system_prompt {
            messages.push(Message {
                role: "system".to_string(),
                content: sys.into(),
            });
        }
        messages.push(Message {
            role: "user".to_string(),
            content: MessageContent::from_chat_text(message),
        });

        let request = ApiChatRequest {
            model: model.to_string(),
            messages,
            temperature: self.effective_temperature(model, temperature),
            stream: Some(options.enabled),
            tools: None,
            tool_choice: None,
        };

        let url = self.chat_completions_url();
        let client = self.http_client();
        let auth_header = self.auth_header.clone();
        let extra_headers = self.extra_headers.clone();
        let openrouter_attribution_headers = self.openrouter_attribution_headers();
        let provider_name = self.name.clone();
        let model_owned = model.to_string();

        let (tx, rx) = tokio::sync::mpsc::channel::<StreamResult<StreamChunk>>(100);

        tokio::spawn(async move {
            let mut req_builder = client.post(&url).json(&request);

            req_builder = match (&auth_header, credential.as_deref()) {
                (AuthStyle::None, _) | (_, None) => req_builder,
                (AuthStyle::Bearer, Some(credential)) => {
                    req_builder.header("Authorization", format!("Bearer {credential}"))
                }
                (AuthStyle::XApiKey, Some(credential)) => {
                    req_builder.header("x-api-key", credential)
                }
                (AuthStyle::Anthropic, Some(credential)) => req_builder
                    .header("x-api-key", credential)
                    .header("anthropic-version", "2023-06-01"),
                (AuthStyle::Custom(header), Some(credential)) => {
                    req_builder.header(header, credential)
                }
            };

            for (name, value) in &extra_headers {
                req_builder = req_builder.header(name.as_str(), value.as_str());
            }
            if let Some((referer, title)) = openrouter_attribution_headers {
                req_builder = req_builder
                    .header("HTTP-Referer", referer)
                    .header("X-OpenRouter-Title", title);
            }

            req_builder = req_builder.header("Accept", "text/event-stream");

            let response = match req_builder.send().await {
                Ok(r) => r,
                Err(e) => {
                    let detail = e.to_string();
                    // F7: a flaky-network timeout / reset / TLS-handshake EOF on
                    // the streaming send is transient transport noise the
                    // socket layer recovers from — gate it so those blips stop
                    // paging Sentry. A non-transient transport failure (DNS
                    // misconfig, unexpected protocol error) still reports.
                    if crate::core::observability::contains_transient_transport_phrase(&detail) {
                        tracing::debug!(
                            domain = "llm_provider",
                            operation = "stream_chat",
                            provider = provider_name.as_str(),
                            model = model_owned.as_str(),
                            failure = "transport",
                            "[llm_provider] stream_chat transient transport error — not reporting to Sentry: {detail}"
                        );
                    } else {
                        crate::core::observability::report_error(
                            detail.as_str(),
                            "llm_provider",
                            "stream_chat",
                            &[
                                ("provider", provider_name.as_str()),
                                ("model", model_owned.as_str()),
                                ("failure", "transport"),
                            ],
                        );
                    }
                    let _ = tx.send(Err(StreamError::Http(e))).await;
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let status_str = status.as_u16().to_string();
                let raw_error = match response.text().await {
                    Ok(e) => e,
                    Err(_) => format!("HTTP error: {}", status),
                };
                let sanitized_error =
                    crate::openhuman::inference::provider::sanitize_api_error(&raw_error);
                let message = format!("{}: {}", status, sanitized_error);
                if crate::openhuman::inference::provider::is_budget_exhausted_http_400(
                    status, &raw_error,
                ) {
                    crate::openhuman::inference::provider::log_budget_exhausted_http_400(
                        "stream_chat",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if crate::openhuman::inference::provider::is_custom_openai_upstream_bad_request_http_400(
                    provider_name.as_str(),
                    status,
                    &raw_error,
                ) {
                    crate::openhuman::inference::provider::log_custom_openai_upstream_bad_request_http_400(
                        "stream_chat",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if crate::openhuman::inference::provider::is_provider_access_policy_denied_http_403(
                    status,
                    &raw_error,
                ) {
                    crate::openhuman::inference::provider::log_provider_access_policy_denied_http_403(
                        "stream_chat",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if crate::openhuman::inference::provider::is_provider_config_rejection_http(
                    status,
                    provider_name.as_str(),
                    &raw_error,
                ) {
                    crate::openhuman::inference::provider::log_provider_config_rejection(
                        "stream_chat",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if crate::openhuman::inference::provider::is_backend_error_code_owned(
                    provider_name.as_str(),
                    &raw_error,
                ) {
                    // F4/F2: managed-backend errorCode (#870) — backend-owned;
                    // the FE must not double-report. Malformed BAD_REQUEST is
                    // excluded and reaches the status gate (it pages — F8).
                    crate::openhuman::inference::provider::log_backend_error_code_owned(
                        "stream_chat",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                        &raw_error,
                    );
                } else if crate::openhuman::inference::provider::is_byo_provider_auth_failure_http(
                    provider_name.as_str(),
                    status,
                    &raw_error,
                ) {
                    crate::openhuman::inference::provider::log_byo_provider_auth_failure(
                        "stream_chat",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if crate::openhuman::inference::provider::is_provider_insufficient_credits_402(
                    status,
                    &raw_error,
                ) {
                    // Insufficient-credits 402: the user's own BYO provider
                    // account is out of balance — a flat billing fact, not a
                    // reservation-window error, so there is NO local max_tokens
                    // lever to apply. Demote to info instead of paging on every
                    // retry; complete classification for a genuinely-
                    // unpreventable BYO-balance condition
                    // (TAURI-RUST-4QF — DeepSeek "Insufficient Balance").
                    crate::openhuman::inference::provider::log_provider_insufficient_credits_402(
                        "stream_chat",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if crate::openhuman::inference::provider::should_report_provider_http_failure(
                    status,
                ) {
                    crate::core::observability::report_error(
                        message.as_str(),
                        "llm_provider",
                        "stream_chat",
                        &[
                            ("provider", provider_name.as_str()),
                            ("model", model_owned.as_str()),
                            ("status", status_str.as_str()),
                            ("failure", "non_2xx"),
                        ],
                    );
                }
                let _ = tx.send(Err(StreamError::Provider(message))).await;
                return;
            }

            let mut chunk_stream = sse_bytes_to_chunks(response, options.count_tokens);
            while let Some(chunk) = chunk_stream.next().await {
                if tx.send(chunk).await.is_err() {
                    break;
                }
            }
        });

        stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|chunk| (chunk, rx))
        })
        .boxed()
    }

    fn stream_chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: f64,
        options: StreamOptions,
    ) -> stream::BoxStream<'static, StreamResult<StreamChunk>> {
        let credential = match self.credential_for_request() {
            Ok(value) => value.map(str::to_string),
            Err(err) => {
                return stream::once(async move { Err(StreamError::Provider(err.to_string())) })
                    .boxed();
            }
        };

        let effective_messages = if self.merge_system_into_user {
            Self::flatten_system_messages(messages)
        } else {
            messages.to_vec()
        };
        let api_messages = effective_messages
            .into_iter()
            .map(|message| Message {
                role: message.role,
                content: MessageContent::from_chat_text(&message.content),
            })
            .collect();

        let request = ApiChatRequest {
            model: model.to_string(),
            messages: api_messages,
            temperature: self.effective_temperature(model, temperature),
            stream: Some(options.enabled),
            tools: None,
            tool_choice: None,
        };

        let url = self.chat_completions_url();
        let client = self.http_client();
        let auth_header = self.auth_header.clone();
        let extra_headers = self.extra_headers.clone();
        let openrouter_attribution_headers = self.openrouter_attribution_headers();
        let provider_name = self.name.clone();
        let model_owned = model.to_string();

        let (tx, rx) = tokio::sync::mpsc::channel::<StreamResult<StreamChunk>>(100);

        tokio::spawn(async move {
            let mut req_builder = client.post(&url).json(&request);
            req_builder = match (&auth_header, credential.as_deref()) {
                (AuthStyle::None, _) | (_, None) => req_builder,
                (AuthStyle::Bearer, Some(credential)) => {
                    req_builder.header("Authorization", format!("Bearer {credential}"))
                }
                (AuthStyle::XApiKey, Some(credential)) => {
                    req_builder.header("x-api-key", credential)
                }
                (AuthStyle::Anthropic, Some(credential)) => req_builder
                    .header("x-api-key", credential)
                    .header("anthropic-version", "2023-06-01"),
                (AuthStyle::Custom(header), Some(credential)) => {
                    req_builder.header(header, credential)
                }
            };
            for (name, value) in &extra_headers {
                req_builder = req_builder.header(name.as_str(), value.as_str());
            }
            if let Some((referer, title)) = openrouter_attribution_headers {
                req_builder = req_builder
                    .header("HTTP-Referer", referer)
                    .header("X-OpenRouter-Title", title);
            }
            req_builder = req_builder.header("Accept", "text/event-stream");

            let response = match req_builder.send().await {
                Ok(response) => response,
                Err(error) => {
                    let detail = error.to_string();
                    // F7: gate transient transport blips (timeout / reset / TLS
                    // handshake EOF) so flaky-network failures on the streaming
                    // send stop paging Sentry; non-transient transport errors
                    // still report.
                    if crate::core::observability::contains_transient_transport_phrase(&detail) {
                        tracing::debug!(
                            domain = "llm_provider",
                            operation = "stream_chat_history",
                            provider = provider_name.as_str(),
                            model = model_owned.as_str(),
                            failure = "transport",
                            "[llm_provider] stream_chat_history transient transport error — not reporting to Sentry: {detail}"
                        );
                    } else {
                        crate::core::observability::report_error(
                            detail.as_str(),
                            "llm_provider",
                            "stream_chat_history",
                            &[
                                ("provider", provider_name.as_str()),
                                ("model", model_owned.as_str()),
                                ("failure", "transport"),
                            ],
                        );
                    }
                    let _ = tx.send(Err(StreamError::Http(error))).await;
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let status_str = status.as_u16().to_string();
                let raw_error = match response.text().await {
                    Ok(error) => error,
                    Err(_) => format!("HTTP error: {status}"),
                };
                let sanitized_error =
                    crate::openhuman::inference::provider::sanitize_api_error(&raw_error);
                let message = format!("{status}: {sanitized_error}");
                if crate::openhuman::inference::provider::is_budget_exhausted_http_400(
                    status, &raw_error,
                ) {
                    crate::openhuman::inference::provider::log_budget_exhausted_http_400(
                        "stream_chat_history",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if crate::openhuman::inference::provider::is_custom_openai_upstream_bad_request_http_400(
                    provider_name.as_str(),
                    status,
                    &raw_error,
                ) {
                    crate::openhuman::inference::provider::log_custom_openai_upstream_bad_request_http_400(
                        "stream_chat_history",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if crate::openhuman::inference::provider::is_provider_access_policy_denied_http_403(
                    status,
                    &raw_error,
                ) {
                    crate::openhuman::inference::provider::log_provider_access_policy_denied_http_403(
                        "stream_chat_history",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if crate::openhuman::inference::provider::is_provider_config_rejection_http(
                    status,
                    provider_name.as_str(),
                    &raw_error,
                ) {
                    crate::openhuman::inference::provider::log_provider_config_rejection(
                        "stream_chat_history",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if crate::openhuman::inference::provider::is_backend_error_code_owned(
                    provider_name.as_str(),
                    &raw_error,
                ) {
                    // F4/F2: managed-backend errorCode (#870) — backend-owned;
                    // the FE must not double-report. Malformed BAD_REQUEST is
                    // excluded and reaches the status gate (it pages — F8).
                    crate::openhuman::inference::provider::log_backend_error_code_owned(
                        "stream_chat_history",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                        &raw_error,
                    );
                } else if crate::openhuman::inference::provider::is_byo_provider_auth_failure_http(
                    provider_name.as_str(),
                    status,
                    &raw_error,
                ) {
                    crate::openhuman::inference::provider::log_byo_provider_auth_failure(
                        "stream_chat_history",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if crate::openhuman::inference::provider::is_provider_insufficient_credits_402(
                    status,
                    &raw_error,
                ) {
                    // Insufficient-credits 402: the user's own BYO provider
                    // account is out of balance — a flat billing fact, not a
                    // reservation-window error, so there is NO local max_tokens
                    // lever to apply. Demote to info instead of paging on every
                    // retry; complete classification for a genuinely-
                    // unpreventable BYO-balance condition
                    // (TAURI-RUST-4QF — DeepSeek "Insufficient Balance").
                    crate::openhuman::inference::provider::log_provider_insufficient_credits_402(
                        "stream_chat_history",
                        provider_name.as_str(),
                        Some(model_owned.as_str()),
                        status,
                    );
                } else if crate::openhuman::inference::provider::should_report_provider_http_failure(
                    status,
                ) {
                    crate::core::observability::report_error(
                        message.as_str(),
                        "llm_provider",
                        "stream_chat_history",
                        &[
                            ("provider", provider_name.as_str()),
                            ("model", model_owned.as_str()),
                            ("status", status_str.as_str()),
                            ("failure", "non_2xx"),
                        ],
                    );
                }
                let _ = tx.send(Err(StreamError::Provider(message))).await;
                return;
            }

            let mut chunk_stream = sse_bytes_to_chunks(response, options.count_tokens);
            while let Some(chunk) = chunk_stream.next().await {
                if tx.send(chunk).await.is_err() {
                    break;
                }
            }
        });

        stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|chunk| (chunk, rx))
        })
        .boxed()
    }

    async fn warmup(&self) -> anyhow::Result<()> {
        if let Some(credential) = self.credential.as_ref() {
            let url = self.chat_completions_url();
            let _ = self
                .apply_auth_header(self.http_client().get(&url), Some(credential.as_str()))
                .send()
                .await?;
        }
        Ok(())
    }

    fn is_local_provider(&self) -> bool {
        self.local_provider_kind.is_some()
    }

    /// Resolve the effective context window for pre-dispatch trimming.
    ///
    /// For cloud (non-local) providers this is the static model table. For
    /// local providers the trained-maximum table over-estimates the window
    /// the runtime actually enforces — LM Studio lets the user load a model
    /// with a smaller `n_ctx`, and budgeting against the max overflows it
    /// (#3550 / Sentry TAURI-RUST-6V0). So for LM Studio we query the native
    /// `/api/v0/models` endpoint for the model's loaded context window, and
    /// fall back to the provider-profile default when it's unavailable.
    async fn effective_context_window(&self, model: &str) -> Option<u64> {
        use crate::openhuman::inference::local::profile::LocalProviderKind;
        let Some(kind) = self.local_provider_kind else {
            return crate::openhuman::inference::context_window_for_model(model);
        };
        // LM Studio: the OpenAI-compat /v1/models hides the context window, but
        // the native /api/v0/models reports the loaded n_ctx. Prefer it.
        if kind == LocalProviderKind::LmStudio {
            if let Some(window) = self.lm_studio_loaded_context_window(model).await {
                return Some(window);
            }
        }
        // Local fallback: pattern table → provider-profile default. Ensures
        // unknown local models still get trimmed instead of skipped.
        crate::openhuman::inference::model_context::context_window_for_model_with_local_fallback(
            model,
            Some(kind),
        )
    }

    /// Authoritative runtime-loaded window: only the value the local runtime
    /// genuinely reports. Today that is LM Studio's native `/api/v0/models`
    /// `loaded_context_length`. For every other case — cloud, llama.cpp / vLLM
    /// (no loaded window endpoint), or a missing probe — we return `None`
    /// rather than a guess, so the engine's hard pre-dispatch abort never fires
    /// on an estimated window (Codex P1 review on PR #3771 / TAURI-RUST-6V0).
    async fn loaded_context_window(&self, model: &str) -> Option<u64> {
        use crate::openhuman::inference::local::profile::LocalProviderKind;
        if self.local_provider_kind == Some(LocalProviderKind::LmStudio) {
            return self.lm_studio_loaded_context_window(model).await;
        }
        None
    }
}

impl OpenAiCompatibleProvider {
    /// Best-effort fetch of the model's loaded context window from LM Studio's
    /// native `/api/v0/models` endpoint. Returns `None` on any transport /
    /// parse error or when the model reports no window — callers then fall
    /// back to the profile default. Never panics, never propagates errors.
    async fn lm_studio_loaded_context_window(&self, model: &str) -> Option<u64> {
        use crate::openhuman::inference::local::lm_studio;
        let url = lm_studio::lm_studio_native_models_url(&self.base_url);
        let resp = match self
            .apply_auth_header(self.http_client().get(&url), self.credential.as_deref())
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(err) => {
                tracing::debug!(
                    provider = %self.name,
                    model,
                    error = %err,
                    "[lm-studio] native models probe transport error; using profile default context window"
                );
                return None;
            }
        };
        if !resp.status().is_success() {
            tracing::debug!(
                provider = %self.name,
                status = resp.status().as_u16(),
                "[lm-studio] native models probe non-success; using profile default context window"
            );
            return None;
        }
        let parsed: lm_studio::LmStudioNativeModelsResponse = match resp.json().await {
            Ok(parsed) => parsed,
            Err(err) => {
                tracing::debug!(
                    provider = %self.name,
                    model,
                    error = %err,
                    "[lm-studio] native models probe parse error; using profile default context window"
                );
                return None;
            }
        };
        let window = lm_studio::lm_studio_context_window_for(&parsed, model);
        tracing::debug!(
            provider = %self.name,
            model,
            loaded_context_window = window.unwrap_or(0),
            "[lm-studio] resolved loaded context window for pre-dispatch trimming"
        );
        window
    }
}
