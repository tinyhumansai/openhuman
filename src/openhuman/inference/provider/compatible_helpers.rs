use crate::openhuman::inference::provider::traits::{
    ChatMessage, ChatResponse as ProviderChatResponse, ToolCall as ProviderToolCall,
    UsageInfo as ProviderUsageInfo,
};

use super::compatible_parse::{
    aggregate_responses_sse_body, build_responses_prompt, extract_responses_text,
    normalize_function_arguments, parse_responses_response_body,
    parse_tool_calls_from_content_json,
};
use super::compatible_types::{
    ApiChatResponse, Message, MessageContent, NativeChatRequest, NativeMessage, ResponsesRequest,
    ToolCall,
};
use super::OpenAiCompatibleProvider;

impl OpenAiCompatibleProvider {
    pub(super) async fn chat_via_responses(
        &self,
        credential: Option<&str>,
        messages: &[ChatMessage],
        model: &str,
        max_output_tokens: Option<u32>,
    ) -> anyhow::Result<String> {
        let (instructions, input) = build_responses_prompt(messages);
        if input.is_empty() {
            anyhow::bail!(
                "{} Responses API fallback requires at least one non-system message",
                self.name
            );
        }

        log::debug!(
            "[provider] {} responses-path model={model} max_output_tokens={:?} input_msgs={}",
            self.name,
            max_output_tokens,
            input.len(),
        );

        // #3201: the Codex/ChatGPT OAuth Responses endpoint rejects `stream: false`
        // outright. This branch lifts the constraint for that endpoint specifically
        // and parses the resulting SSE body so the existing non-streaming call
        // signature is preserved. Other providers keep the single-envelope path.
        let is_codex_oauth_responses = reqwest::Url::parse(&self.base_url)
            .ok()
            .and_then(|url| {
                let segments: Vec<&str> = url.path_segments()?.collect();
                Some(
                    segments
                        .windows(2)
                        .any(|window| window == ["backend-api", "codex"]),
                )
            })
            .unwrap_or(false);

        let request = ResponsesRequest {
            model: model.to_string(),
            input,
            instructions,
            stream: Some(is_codex_oauth_responses),
            store: Some(false),
            max_output_tokens,
        };

        let url = self.responses_url();

        let response = self
            .apply_auth_header(self.http_client().post(&url).json(&request), credential)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let status_str = status.as_u16().to_string();
            let error = response.text().await?;
            let sanitized = super::super::sanitize_api_error(&error);
            let message = format!("{} Responses API error: {sanitized}", self.name);
            if super::super::is_budget_exhausted_http_400(status, &error) {
                super::super::log_budget_exhausted_http_400(
                    "responses_api",
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
                    "responses_api",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::super::is_provider_access_policy_denied_http_403(status, &error) {
                super::super::log_provider_access_policy_denied_http_403(
                    "responses_api",
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
                    "responses_api",
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
                    "responses_api",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::super::is_openai_oauth_session_expired_http(
                self.name.as_str(),
                status,
                &error,
            ) {
                super::super::log_openai_oauth_session_expired(
                    "responses_api",
                    self.name.as_str(),
                    Some(model),
                    status,
                );
            } else if super::super::should_report_provider_http_failure(status) {
                crate::core::observability::report_error(
                    message.as_str(),
                    "llm_provider",
                    "responses_api",
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
        if is_codex_oauth_responses {
            return aggregate_responses_sse_body(&self.name, &body);
        }
        let responses = parse_responses_response_body(&self.name, &body)?;

        extract_responses_text(responses)
            .ok_or_else(|| anyhow::anyhow!("No response from {} Responses API", self.name))
    }

    pub(super) fn convert_tool_specs(
        tools: Option<&[crate::openhuman::tools::ToolSpec]>,
    ) -> Option<Vec<serde_json::Value>> {
        tools.map(|items| {
            let mut seen: std::collections::HashSet<&str> =
                std::collections::HashSet::with_capacity(items.len());
            let mut dropped: Vec<&str> = Vec::new();
            let mut out: Vec<serde_json::Value> = Vec::with_capacity(items.len());
            for tool in items {
                if !seen.insert(tool.name.as_str()) {
                    dropped.push(tool.name.as_str());
                    continue;
                }
                out.push(serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                }));
            }
            if !dropped.is_empty() {
                log::warn!(
                    "[providers][compatible] dropped {} duplicate tool spec(s) at wire \
                     boundary (TAURI-RUST-2E): {:?}",
                    dropped.len(),
                    dropped
                );
            }
            out
        })
    }

    pub(super) fn convert_messages_for_native(messages: &[ChatMessage]) -> Vec<NativeMessage> {
        let converted: Vec<NativeMessage> =
            messages
                .iter()
                .map(|message| {
                    let reasoning_content = if message.role == "assistant" {
                        message
                            .extra_metadata
                            .as_ref()
                            .and_then(|m| m.get("reasoning_content"))
                            .and_then(serde_json::Value::as_str)
                            .map(ToString::to_string)
                    } else {
                        None
                    };

                    if message.role == "assistant" {
                        if let Ok(value) =
                            serde_json::from_str::<serde_json::Value>(&message.content)
                        {
                            if let Some(tool_calls_value) = value.get("tool_calls") {
                                if let Ok(parsed_calls) =
                                    serde_json::from_value::<Vec<ProviderToolCall>>(
                                        tool_calls_value.clone(),
                                    )
                                {
                                    let tool_calls = parsed_calls
                                        .into_iter()
                                        .map(|tc| ToolCall {
                                            id: Some(tc.id),
                                            kind: Some("function".to_string()),
                                            function: Some(super::compatible_types::Function {
                                                name: Some(tc.name),
                                                arguments: Some(serde_json::Value::String(
                                                    tc.arguments,
                                                )),
                                            }),
                                            // Echo Gemini's thought_signature back on
                                            // the next turn (TAURI-RUST-4PK).
                                            extra_content: tc.extra_content,
                                        })
                                        .collect::<Vec<_>>();

                                    let content = Some(MessageContent::Text(
                                        value
                                            .get("content")
                                            .and_then(serde_json::Value::as_str)
                                            .unwrap_or("")
                                            .to_string(),
                                    ));

                                    let reasoning_content = value
                                        .get("reasoning_content")
                                        .and_then(serde_json::Value::as_str)
                                        .filter(|s| !s.trim().is_empty())
                                        .map(ToString::to_string)
                                        .or_else(|| reasoning_content.clone());

                                    return NativeMessage {
                                        role: "assistant".to_string(),
                                        content,
                                        tool_call_id: None,
                                        tool_calls: Some(tool_calls),
                                        reasoning_content,
                                    };
                                }
                            }
                        }
                    }

                    if message.role == "tool" {
                        if let Ok(value) =
                            serde_json::from_str::<serde_json::Value>(&message.content)
                        {
                            let tool_call_id = value
                                .get("tool_call_id")
                                .and_then(serde_json::Value::as_str)
                                .map(ToString::to_string);
                            let content = value
                                .get("content")
                                .and_then(serde_json::Value::as_str)
                                .map(ToString::to_string)
                                .or_else(|| Some(message.content.clone()))
                                .map(MessageContent::Text);

                            return NativeMessage {
                                role: "tool".to_string(),
                                content,
                                tool_call_id,
                                tool_calls: None,
                                reasoning_content: None,
                            };
                        }
                    }

                    NativeMessage {
                        role: message.role.clone(),
                        content: Some(MessageContent::from_chat_text(&message.content)),
                        tool_call_id: None,
                        tool_calls: None,
                        reasoning_content,
                    }
                })
                .collect();

        Self::enforce_tool_message_invariants(converted)
    }

    /// Enforce the OpenAI-compatible tool-message ordering invariants on the
    /// fully-serialized wire array, immediately before it goes on the wire.
    ///
    /// Several upstream defects can leave the array malformed and trip a 400
    /// (`messages with role 'tool' must be a response to a preceding message
    /// with 'tool_calls'`). That 400 streams back as an empty completion, which
    /// the agent loop collapses to "The model returned an empty response" and
    /// the chat surface shows as a generic "Something went wrong":
    ///
    /// * **(A)** History tail-trimming cuts *between* an `assistant(tool_calls)`
    ///   and its `tool` result, dropping the assistant and orphaning the result.
    /// * **(B)** A persisted assistant tool-call message whose `content` no
    ///   longer deserializes as `tool_calls` (format drift) falls through and
    ///   is emitted as plain text with its `tool_calls` stripped.
    /// * **(C)** An `assistant(tool_calls)` whose results never arrived leaves
    ///   dangling tool-call ids with no matching `tool` response.
    pub(super) fn enforce_tool_message_invariants(
        messages: Vec<NativeMessage>,
    ) -> Vec<NativeMessage> {
        use std::collections::HashSet;

        let mut out: Vec<NativeMessage> = Vec::with_capacity(messages.len());
        let mut dropped_orphans = 0usize;
        let mut pruned_calls = 0usize;

        let mut iter = messages.into_iter().peekable();
        while let Some(mut msg) = iter.next() {
            if msg.role == "assistant" && msg.tool_calls.is_some() {
                let mut run: Vec<NativeMessage> = Vec::new();
                while iter.peek().is_some_and(|m| m.role == "tool") {
                    run.push(iter.next().expect("peeked tool message"));
                }
                let responded: HashSet<String> =
                    run.iter().filter_map(|t| t.tool_call_id.clone()).collect();

                let calls = msg.tool_calls.take().unwrap_or_default();
                let before = calls.len();
                let kept: Vec<ToolCall> = calls
                    .into_iter()
                    .filter(|c| c.id.as_deref().is_some_and(|id| responded.contains(id)))
                    .collect();
                pruned_calls += before - kept.len();
                let kept_ids: HashSet<String> = kept.iter().filter_map(|c| c.id.clone()).collect();
                msg.tool_calls = if kept.is_empty() { None } else { Some(kept) };
                if msg.tool_calls.is_none() {
                    msg.reasoning_content = None;
                }
                out.push(msg);

                for tool_msg in run {
                    let kept = tool_msg
                        .tool_call_id
                        .as_deref()
                        .is_some_and(|id| kept_ids.contains(id));
                    if kept {
                        out.push(tool_msg);
                    } else {
                        dropped_orphans += 1;
                    }
                }
            } else if msg.role == "tool" {
                dropped_orphans += 1;
            } else {
                out.push(msg);
            }
        }

        if dropped_orphans > 0 || pruned_calls > 0 {
            log::warn!(
                "[provider] sanitized malformed tool-message ordering before send: \
                 dropped {dropped_orphans} orphaned tool result(s), pruned {pruned_calls} \
                 unanswered tool_call(s)"
            );
        }

        out
    }

    pub(super) fn with_prompt_guided_tool_instructions(
        messages: &[ChatMessage],
        tools: Option<&[crate::openhuman::tools::ToolSpec]>,
    ) -> Vec<ChatMessage> {
        let Some(tools) = tools else {
            return messages.to_vec();
        };

        if tools.is_empty() {
            return messages.to_vec();
        }

        let instructions =
            crate::openhuman::inference::provider::traits::build_tool_instructions_text(tools);
        let mut modified_messages = messages.to_vec();

        if let Some(system_message) = modified_messages.iter_mut().find(|m| m.role == "system") {
            if !system_message.content.is_empty() {
                system_message.content.push_str("\n\n");
            }
            system_message.content.push_str(&instructions);
        } else {
            modified_messages.insert(0, ChatMessage::system(instructions));
        }

        modified_messages
    }

    pub(super) fn parse_native_response(
        api_response: ApiChatResponse,
        provider_name: &str,
    ) -> anyhow::Result<ProviderChatResponse> {
        let usage = Self::extract_usage(&api_response);

        let message = api_response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message)
            .ok_or_else(|| anyhow::anyhow!("No choices in response from {}", provider_name))?;

        let mut text = message.effective_content_optional();
        let reasoning_content = message
            .reasoning_content
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let mut tool_calls = message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .filter_map(|tc| {
                let function = tc.function?;
                let name = function.name?;
                let arguments = normalize_function_arguments(function.arguments);
                Some(ProviderToolCall {
                    id: tc.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                    name,
                    arguments,
                    // Preserve Gemini's thought_signature (TAURI-RUST-4PK) so it
                    // can be echoed on the next turn; None for providers that
                    // don't send extra_content.
                    extra_content: tc.extra_content,
                })
            })
            .collect::<Vec<_>>();

        if tool_calls.is_empty() {
            if let Some(function) = message.function_call.as_ref() {
                if let Some(name) = function
                    .name
                    .as_ref()
                    .filter(|name| !name.trim().is_empty())
                {
                    tool_calls.push(ProviderToolCall {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: name.clone(),
                        arguments: normalize_function_arguments(function.arguments.clone()),
                        // Legacy `function_call` shape carries no extra_content.
                        extra_content: None,
                    });
                }
            }
        }

        if let Some(content) = message.content.as_deref() {
            if let Some((json_text, json_tool_calls)) = parse_tool_calls_from_content_json(content)
            {
                if !json_tool_calls.is_empty() {
                    tool_calls = json_tool_calls;
                    text = json_text.or(text);
                }
            }
        }

        tracing::debug!(
            has_reasoning_content = reasoning_content.is_some(),
            reasoning_content_chars = reasoning_content.as_ref().map_or(0, |r| r.chars().count()),
            "[provider:parse_native_response] reasoning_content capture"
        );

        Ok(ProviderChatResponse {
            text,
            tool_calls,
            usage,
            reasoning_content,
        })
    }

    /// Extract usage info from API response, preferring the OpenHuman
    /// metadata block (which includes cache stats and billing) over the
    /// standard OpenAI usage block.
    pub(super) fn extract_usage(resp: &ApiChatResponse) -> Option<ProviderUsageInfo> {
        let oh = resp.openhuman.as_ref();
        let std_usage = resp.usage.as_ref();

        if oh.is_none() && std_usage.is_none() {
            return None;
        }

        let oh_usage = oh.and_then(|o| o.usage.as_ref());
        let oh_billing = oh.and_then(|o| o.billing.as_ref());

        let input_tokens = oh_usage
            .and_then(|u| u.input_tokens)
            .or(std_usage.map(|u| u.prompt_tokens))
            .unwrap_or(0);
        let output_tokens = oh_usage
            .and_then(|u| u.output_tokens)
            .or(std_usage.map(|u| u.completion_tokens))
            .unwrap_or(0);
        let cached_input_tokens = oh_usage
            .and_then(|u| u.cached_input_tokens)
            .or(std_usage
                .and_then(|u| u.prompt_tokens_details.as_ref())
                .map(|d| d.cached_tokens))
            .unwrap_or(0);
        let charged_amount_usd = oh_billing.map(|b| b.charged_amount_usd).unwrap_or(0.0);

        let from_openhuman = oh_usage.is_some();
        let from_standard = std_usage.is_some() && !from_openhuman;
        let has_billing = oh_billing.is_some();
        tracing::debug!(
            from_openhuman,
            from_standard,
            has_billing,
            input_tokens,
            output_tokens,
            cached_input_tokens,
            charged_amount_usd,
            "[provider:usage] extract_usage resolved token counts"
        );

        Some(ProviderUsageInfo {
            input_tokens,
            output_tokens,
            context_window: 0,
            cached_input_tokens,
            charged_amount_usd,
        })
    }

    pub(super) fn is_native_tool_schema_unsupported(
        status: reqwest::StatusCode,
        error: &str,
    ) -> bool {
        if !matches!(
            status,
            reqwest::StatusCode::BAD_REQUEST | reqwest::StatusCode::UNPROCESSABLE_ENTITY
        ) {
            return false;
        }

        let lower = error.to_lowercase();
        [
            "unknown parameter: tools",
            "unsupported parameter: tools",
            "unrecognized field `tools`",
            "does not support tools",
            "function calling is not supported",
            "tool_choice",
        ]
        .iter()
        .any(|hint| lower.contains(hint))
    }

    pub(super) fn err_supports_no_tools_retry(error: &str) -> bool {
        Self::is_native_tool_schema_unsupported(reqwest::StatusCode::BAD_REQUEST, error)
    }

    /// Detect a provider rejecting the `frequency_penalty` sampling field. Some
    /// strict OpenAI-compatible backends 400 on unknown params; when this fires
    /// the caller retries once with the field omitted (mirrors the no-tools
    /// retry). String-based because the streamed transport error surfaces the
    /// API error body.
    pub(super) fn err_indicates_frequency_penalty_unsupported(error: &str) -> bool {
        let lower = error.to_lowercase();
        lower.contains("frequency_penalty")
            && (lower.contains("unsupported")
                || lower.contains("unknown")
                || lower.contains("unrecognized")
                || lower.contains("not supported")
                || lower.contains("does not support")
                || lower.contains("invalid")
                || lower.contains("unexpected"))
    }

    /// Detect a 404 whose body says the model is completion-only. See issue #3193.
    pub(super) fn is_completion_only_model_404(status: reqwest::StatusCode, error: &str) -> bool {
        if status != reqwest::StatusCode::NOT_FOUND {
            return false;
        }
        let lower = error.to_lowercase();
        lower.contains("not a chat model")
            || (lower.contains("v1/chat/completions") && lower.contains("v1/completions"))
    }

    /// Detect a model rejected because it has no chat capability. See Sentry TAURI-RUST-4P6.
    pub(super) fn is_not_chat_capable_model(status: reqwest::StatusCode, error: &str) -> bool {
        if !matches!(
            status,
            reqwest::StatusCode::BAD_REQUEST | reqwest::StatusCode::UNPROCESSABLE_ENTITY
        ) {
            return false;
        }
        error.to_lowercase().contains("does not support chat")
    }

    pub(super) fn completion_only_model_message(&self, model: &str, sanitized: &str) -> String {
        format!(
            "{name} API error (404): model '{model}' does not support the \
             chat-completions API that OpenHuman uses — it appears to be a \
             completion-only / base model. Assign a chat-capable model to this \
             provider (e.g. in Settings → AI), or pick a different model. \
             Provider detail: {sanitized}",
            name = self.name,
        )
    }

    /// Guard shared by every chat-completions 404 handler. See issue #3193.
    pub(super) fn completion_only_404_guard(
        &self,
        status: reqwest::StatusCode,
        sanitized: &str,
        model: &str,
    ) -> Option<anyhow::Error> {
        if Self::is_completion_only_model_404(status, sanitized) {
            Some(anyhow::anyhow!(
                self.completion_only_model_message(model, sanitized)
            ))
        } else {
            None
        }
    }

    pub(super) fn not_chat_capable_model_message(&self, model: &str, sanitized: &str) -> String {
        format!(
            "{name} API error: model '{model}' does not support chat — it \
             appears to be an embedding or non-chat model. Assign a \
             chat-capable model to this provider (e.g. in Settings → AI), or \
             pick a different model. Provider detail: {sanitized}",
            name = self.name,
        )
    }

    /// Guard shared by every chat-completions error handler. See Sentry TAURI-RUST-4P6.
    pub(super) fn not_chat_capable_guard(
        &self,
        status: reqwest::StatusCode,
        sanitized: &str,
        model: &str,
    ) -> Option<anyhow::Error> {
        if Self::is_not_chat_capable_model(status, sanitized) {
            Some(anyhow::anyhow!(
                self.not_chat_capable_model_message(model, sanitized)
            ))
        } else {
            None
        }
    }

    pub(super) fn enrich_404_message(&self, base: String, status: reqwest::StatusCode) -> String {
        if status == reqwest::StatusCode::NOT_FOUND && !self.supports_responses_fallback {
            format!(
                "{base}; check that your endpoint URL is correct \
                 and the model name exists on your provider"
            )
        } else {
            base
        }
    }
}
