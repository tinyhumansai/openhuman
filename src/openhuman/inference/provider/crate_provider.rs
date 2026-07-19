//! Legacy [`Provider`] boundary backed by a crate-native [`ChatModel`].

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use futures_util::StreamExt;
use tinyagents::harness::model::{ChatModel, ModelRequest, ModelResponse, ModelStreamItem};

use super::traits::{
    ChatMessage, ChatRequest, ChatResponse, Provider, ProviderCapabilities, ProviderDelta,
    ToolCall, ToolsPayload,
};
use crate::openhuman::tools::ToolSpec;

fn sanitize_model_error(message: &str) -> String {
    if message.contains("-----BEGIN") {
        return "provider error contained sensitive content [redacted]".to_string();
    }

    static PATTERNS: OnceLock<Vec<regex::Regex>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        [
            r"(?i)\bBearer\s+[^\s,;]+",
            r"\bsk-[A-Za-z0-9_-]{8,}\b",
            r"\bgh[pousr]_[A-Za-z0-9_]{8,}\b",
        ]
        .into_iter()
        .map(|pattern| regex::Regex::new(pattern).expect("valid provider error redaction regex"))
        .collect()
    });
    patterns
        .iter()
        .fold(message.to_string(), |sanitized, pattern| {
            pattern.replace_all(&sanitized, "[REDACTED]").into_owned()
        })
}

pub(crate) struct CrateBackedProvider {
    model: Arc<dyn ChatModel<()>>,
    provider_id: String,
    local: bool,
}

impl CrateBackedProvider {
    pub(crate) fn new(model: Arc<dyn ChatModel<()>>, provider_id: impl Into<String>) -> Self {
        Self {
            model,
            provider_id: provider_id.into(),
            local: false,
        }
    }

    pub(crate) fn with_local(mut self) -> Self {
        self.local = true;
        self
    }

    fn request(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSpec]>,
        model: &str,
        temperature: f64,
        max_tokens: Option<u32>,
    ) -> ModelRequest {
        let mut request = ModelRequest::new(
            messages
                .iter()
                .map(crate::openhuman::tinyagents::chat_message_to_message)
                .collect(),
        )
        .with_model(model.to_string())
        .with_temperature(temperature);
        request.tools = tools
            .unwrap_or_default()
            .iter()
            .map(crate::openhuman::tinyagents::spec_to_schema)
            .collect();
        request.max_tokens = max_tokens;
        request
    }

    async fn invoke(&self, request: ModelRequest) -> anyhow::Result<ChatResponse> {
        tracing::debug!(
            provider = %self.provider_id,
            model = request.model.as_deref().unwrap_or("<default>"),
            tool_count = request.tools.len(),
            "[inference][crate-provider] invoking crate-native model through legacy boundary"
        );
        let response = match self.model.invoke(&(), request).await {
            Ok(response) => response,
            Err(error) => {
                let message = error.to_string();
                if self.provider_id.eq_ignore_ascii_case("openhuman")
                    && (message.contains("HTTP 401") || message.contains("HTTP 403"))
                {
                    let status = if message.contains("HTTP 401") {
                        reqwest::StatusCode::UNAUTHORIZED
                    } else {
                        reqwest::StatusCode::FORBIDDEN
                    };
                    super::ops::publish_backend_session_expired(
                        "crate_model",
                        &self.provider_id,
                        status,
                        &message,
                    );
                }
                return Err(anyhow::anyhow!(sanitize_model_error(&message)));
            }
        };
        Ok(Self::response(response))
    }

    fn response(response: ModelResponse) -> ChatResponse {
        let reasoning_content =
            crate::openhuman::tinyagents::reasoning_from_content(&response.message.content);
        ChatResponse {
            text: Some(response.text()),
            tool_calls: response
                .message
                .tool_calls
                .iter()
                .map(|call| ToolCall {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: call.arguments.to_string(),
                    extra_content: None,
                })
                .collect(),
            usage: crate::openhuman::tinyagents::model::usage_info_from_response(&response),
            reasoning_content,
        }
    }
}

#[async_trait]
impl Provider for CrateBackedProvider {
    fn telemetry_provider_id(&self) -> String {
        self.provider_id.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.model
            .profile()
            .map(|profile| ProviderCapabilities {
                native_tool_calling: profile.tool_calling,
                vision: profile.modalities.image_in,
            })
            .unwrap_or_default()
    }

    fn convert_tools(&self, tools: &[ToolSpec]) -> ToolsPayload {
        ToolsPayload::OpenAI {
            tools: tools
                .iter()
                .map(|tool| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        }
                    })
                })
                .collect(),
        }
    }

    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let mut messages = Vec::new();
        if let Some(system) = system_prompt {
            messages.push(ChatMessage::system(system));
        }
        messages.push(ChatMessage::user(message));
        Ok(self
            .invoke(self.request(&messages, None, model, temperature, None))
            .await?
            .text
            .unwrap_or_default())
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        Ok(self
            .invoke(self.request(messages, None, model, temperature, None))
            .await?
            .text
            .unwrap_or_default())
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        let model_request = self.request(
            request.messages,
            request.tools,
            model,
            temperature,
            request.max_tokens,
        );
        let Some(delta_tx) = request.stream else {
            return self.invoke(model_request).await;
        };

        let mut stream = self
            .model
            .stream(&(), model_request)
            .await
            .map_err(|error| anyhow::anyhow!(sanitize_model_error(&error.to_string())))?;
        let mut completed = None;
        let mut started_tool_calls = HashSet::new();
        while let Some(item) = stream.next().await {
            match item {
                ModelStreamItem::MessageDelta(delta) => {
                    if !delta.text.is_empty() {
                        let _ = delta_tx
                            .send(ProviderDelta::TextDelta { delta: delta.text })
                            .await;
                    }
                    if !delta.reasoning.is_empty() {
                        let _ = delta_tx
                            .send(ProviderDelta::ThinkingDelta {
                                delta: delta.reasoning,
                            })
                            .await;
                    }
                    if let Some(tool) = delta.tool_call {
                        if let Some(tool_name) = tool.tool_name {
                            if started_tool_calls.insert(tool.call_id.clone()) {
                                let _ = delta_tx
                                    .send(ProviderDelta::ToolCallStart {
                                        call_id: tool.call_id.clone(),
                                        tool_name,
                                    })
                                    .await;
                            }
                        }
                        if !tool.content.is_empty() {
                            let _ = delta_tx
                                .send(ProviderDelta::ToolCallArgsDelta {
                                    call_id: tool.call_id,
                                    delta: tool.content,
                                })
                                .await;
                        }
                    }
                }
                ModelStreamItem::ToolCallDelta(tool) => {
                    if let Some(tool_name) = tool.tool_name {
                        if started_tool_calls.insert(tool.call_id.clone()) {
                            let _ = delta_tx
                                .send(ProviderDelta::ToolCallStart {
                                    call_id: tool.call_id.clone(),
                                    tool_name,
                                })
                                .await;
                        }
                    }
                    if !tool.content.is_empty() {
                        let _ = delta_tx
                            .send(ProviderDelta::ToolCallArgsDelta {
                                call_id: tool.call_id,
                                delta: tool.content,
                            })
                            .await;
                    }
                }
                ModelStreamItem::Completed(response) => completed = Some(response),
                ModelStreamItem::Failed(error) => {
                    anyhow::bail!(sanitize_model_error(&error))
                }
                ModelStreamItem::ProviderFailed(error) => {
                    anyhow::bail!(sanitize_model_error(&error.to_string()))
                }
                ModelStreamItem::Started | ModelStreamItem::UsageDelta(_) => {}
            }
        }
        completed
            .map(Self::response)
            .ok_or_else(|| anyhow::anyhow!("crate model stream ended without a completed response"))
    }

    fn supports_streaming(&self) -> bool {
        self.model.profile().is_none_or(|profile| profile.streaming)
    }

    fn is_local_provider(&self) -> bool {
        self.local
    }

    fn is_local_provider_for_model(&self, _model: &str) -> bool {
        self.local
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_model_error;

    #[test]
    fn model_error_sanitizer_redacts_credentials_and_preserves_safe_errors() {
        assert_eq!(
            sanitize_model_error("HTTP 403: denied for sk-provider-secret"),
            "HTTP 403: denied for [REDACTED]"
        );
        assert_eq!(
            sanitize_model_error("HTTP 404: missing"),
            "HTTP 404: missing"
        );
    }
}
