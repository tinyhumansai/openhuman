use crate::openhuman::inference::provider::traits::ChatResponse as ProviderChatResponse;

use super::compatible_dump::dump_response_if_enabled;
use super::compatible_repeat::{StreamRepeatDetector, STREAM_REPEAT_THRESHOLD};
use super::compatible_types::{
    ApiChatResponse, ApiUsage, Choice, Function, NativeChatRequest, OpenHumanMeta, ResponseMessage,
    StreamChunkResponse, StreamingToolCall, ToolCall,
};
use super::OpenAiCompatibleProvider;

impl OpenAiCompatibleProvider {
    /// Streaming variant of the native-tools chat path.
    ///
    /// Sends the request with `stream: true`, consumes the upstream SSE
    /// stream chunk by chunk, forwards fine-grained `ProviderDelta`
    /// events to the caller-supplied sender, and returns the aggregated
    /// [`ProviderChatResponse`] once the stream ends.
    pub(super) async fn stream_native_chat(
        &self,
        credential: Option<&str>,
        native_request: &NativeChatRequest,
        delta_tx: &tokio::sync::mpsc::Sender<crate::openhuman::inference::provider::ProviderDelta>,
        dump_seq: u64,
    ) -> anyhow::Result<ProviderChatResponse> {
        use futures_util::StreamExt;

        let url = self.chat_completions_url();
        log::info!(
            "[stream] {} POST {} (stream=true, tools={})",
            self.name,
            url,
            native_request.tools.as_ref().map_or(0, |t| t.len()),
        );

        let response = self
            .apply_auth_header(
                self.http_client()
                    .post(&url)
                    .header("Accept", "text/event-stream")
                    .json(native_request),
                credential,
            )
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let status_str = status.as_u16().to_string();
            let body = response.text().await.unwrap_or_default();
            let sanitized = super::super::sanitize_api_error(&body);
            let message = format!(
                "{} streaming API error ({}): {}",
                self.name, status, sanitized
            );
            if super::super::is_budget_exhausted_http_400(status, &body) {
                super::super::log_budget_exhausted_http_400(
                    "streaming_chat",
                    self.name.as_str(),
                    Some(native_request.model.as_str()),
                    status,
                );
            } else if super::super::is_custom_openai_upstream_bad_request_http_400(
                self.name.as_str(),
                status,
                &body,
            ) {
                super::super::log_custom_openai_upstream_bad_request_http_400(
                    "streaming_chat",
                    self.name.as_str(),
                    Some(native_request.model.as_str()),
                    status,
                );
            } else if super::super::is_provider_access_policy_denied_http_403(status, &body) {
                super::super::log_provider_access_policy_denied_http_403(
                    "streaming_chat",
                    self.name.as_str(),
                    Some(native_request.model.as_str()),
                    status,
                );
            } else if super::super::is_provider_config_rejection_http(
                status,
                self.name.as_str(),
                &body,
            ) {
                super::super::log_provider_config_rejection(
                    "streaming_chat",
                    self.name.as_str(),
                    Some(native_request.model.as_str()),
                    status,
                );
            } else if Self::is_native_tool_schema_unsupported(status, &body) {
                log::info!(
                    "[stream] {} model rejected tool schema (status={}) — caller will retry without tools",
                    self.name,
                    status,
                );
            } else if Self::err_indicates_frequency_penalty_unsupported(&body) {
                // Endpoint rejects `frequency_penalty` (e.g. an unknown strict
                // provider not yet covered by `effective_frequency_penalty`).
                // The caller retries without the field and succeeds, so this is
                // a self-healed recoverable condition — log, don't page
                // (TAURI-RUST-4PJ). Defense-in-depth behind the prevent-at-source
                // omission; the bail! below still drives the retry path.
                log::info!(
                    "[stream] {} rejected frequency_penalty (status={}) — caller will retry without it",
                    self.name,
                    status,
                );
            } else if super::super::is_backend_error_code_owned(self.name.as_str(), &body) {
                // F4/F2: managed-backend errorCode (#870) — backend-owned, FE
                // must not double-report. Malformed BAD_REQUEST is excluded and
                // falls through to the status gate below.
                super::super::log_backend_error_code_owned(
                    "streaming_chat",
                    self.name.as_str(),
                    Some(native_request.model.as_str()),
                    status,
                    &body,
                );
            } else if super::super::should_report_provider_http_failure(status) {
                crate::core::observability::report_error(
                    message.as_str(),
                    "llm_provider",
                    "streaming_chat",
                    &[
                        ("provider", self.name.as_str()),
                        ("model", native_request.model.as_str()),
                        ("status", status_str.as_str()),
                        ("failure", "non_2xx"),
                    ],
                );
            }
            anyhow::bail!(message);
        }

        let is_sse = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|ct| ct.to_ascii_lowercase().contains("text/event-stream"))
            .unwrap_or(false);
        if !is_sse {
            log::warn!(
                "[stream] {} upstream replied with non-SSE content-type; falling back to JSON parse \
                 (no token deltas reach the UI)",
                self.name,
            );
            let response_bytes = response.bytes().await?;
            dump_response_if_enabled(&self.name, &native_request.model, dump_seq, &response_bytes);
            let api_resp: ApiChatResponse = serde_json::from_slice(&response_bytes)
                .map_err(|err| anyhow::anyhow!("{} response parse error: {err}", self.name))?;
            return Self::parse_native_response(api_resp, &self.name);
        }

        let mut text_accum = String::new();
        let mut thinking_accum = String::new();
        let mut tool_accum: std::collections::BTreeMap<u32, StreamingToolCall> =
            std::collections::BTreeMap::new();
        let mut last_usage: Option<ApiUsage> = None;
        let mut last_openhuman: Option<OpenHumanMeta> = None;

        let mut bytes_stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut repeat_detector = StreamRepeatDetector::new();
        let mut degenerate_repeat = false;

        'stream: while let Some(item) = bytes_stream.next().await {
            let bytes = item?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            while let Some(sep_idx) = buffer.find("\n\n") {
                let event = buffer[..sep_idx].to_string();
                buffer.drain(..sep_idx + 2);
                for line in event.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with(':') {
                        continue;
                    }
                    let Some(data) = line.strip_prefix("data:") else {
                        continue;
                    };
                    let data = data.trim();
                    if data == "[DONE]" {
                        continue;
                    }

                    let chunk: StreamChunkResponse = match serde_json::from_str(data) {
                        Ok(v) => v,
                        Err(e) => {
                            log::debug!(
                                "[stream] {} skipping unparseable chunk: {} — data={}",
                                self.name,
                                e,
                                data,
                            );
                            continue;
                        }
                    };

                    if let Some(usage) = chunk.usage {
                        last_usage = Some(usage);
                    }
                    if let Some(meta) = chunk.openhuman {
                        last_openhuman = Some(meta);
                    }

                    for choice in chunk.choices {
                        if let Some(content) = choice.delta.content.as_ref() {
                            if !content.is_empty() {
                                text_accum.push_str(content);
                                let _ = delta_tx
                                    .send(crate::openhuman::inference::provider::ProviderDelta::TextDelta {
                                        delta: content.clone(),
                                    })
                                    .await;
                                if repeat_detector.observe(content) {
                                    log::warn!(
                                        "[stream] {} degenerate repetition detected (≥{} identical lines) — aborting generation, truncating (text_chars={})",
                                        self.name,
                                        STREAM_REPEAT_THRESHOLD,
                                        text_accum.chars().count(),
                                    );
                                    degenerate_repeat = true;
                                    break 'stream;
                                }
                            }
                        }
                        if let Some(reasoning) = choice.delta.reasoning_content.as_ref() {
                            if !reasoning.is_empty() {
                                thinking_accum.push_str(reasoning);
                                let _ = delta_tx
                                    .send(
                                        crate::openhuman::inference::provider::ProviderDelta::ThinkingDelta {
                                            delta: reasoning.clone(),
                                        },
                                    )
                                    .await;
                            }
                        }
                        // Tool-call fragments.
                        //
                        // Ordering invariant emitted downstream:
                        //   ToolCallStart (once, when id+name both known)
                        //     → ToolCallArgsDelta* (buffered then streamed)
                        //
                        // Args fragments that arrive *before* we know the
                        // canonical id are buffered but NOT emitted — emitting
                        // them with a synthetic id would break client-side
                        // reconciliation. Once start fires we flush the buffered
                        // prefix in a single delta, then stream subsequent
                        // fragments as they arrive.
                        if let Some(tc_list) = choice.delta.tool_calls.as_ref() {
                            for tc in tc_list {
                                let idx = tc.index.unwrap_or(0);
                                let entry = tool_accum.entry(idx).or_default();

                                // Capture the first non-null extra_content seen for
                                // this index (Gemini's thought_signature, TAURI-RUST-4PK).
                                if entry.extra_content.is_none() {
                                    if let Some(ec) = tc.extra_content.as_ref() {
                                        if !ec.is_null() {
                                            entry.extra_content = Some(ec.clone());
                                        }
                                    }
                                }

                                if let Some(id) = tc.id.as_ref() {
                                    if entry.id.is_none() {
                                        log::debug!(
                                            "[stream] {} tool_call[{}] id resolved: {}",
                                            self.name,
                                            idx,
                                            id,
                                        );
                                    }
                                    entry.id = Some(id.clone());
                                }
                                if let Some(func) = tc.function.as_ref() {
                                    if let Some(name) = func.name.as_ref() {
                                        if !name.is_empty() && entry.name.is_none() {
                                            log::debug!(
                                                "[stream] {} tool_call[{}] name resolved: {}",
                                                self.name,
                                                idx,
                                                name,
                                            );
                                        }
                                        if !name.is_empty() {
                                            entry.name = Some(name.clone());
                                        }
                                    }
                                    if let Some(args) = func.arguments.as_ref() {
                                        if !args.is_empty() {
                                            entry.arguments.push_str(args);
                                            if !entry.emitted_start {
                                                log::debug!(
                                                    "[stream] {} tool_call[{}] buffering args ({} chars total) — waiting for id/name",
                                                    self.name,
                                                    idx,
                                                    entry.arguments.len(),
                                                );
                                            }
                                        }
                                    }
                                }

                                if !entry.emitted_start {
                                    if let (Some(id), Some(name)) =
                                        (entry.id.as_ref(), entry.name.as_ref())
                                    {
                                        log::debug!(
                                            "[stream] {} tool_call[{}] emitting ToolCallStart id={} name={}",
                                            self.name,
                                            idx,
                                            id,
                                            name,
                                        );
                                        let _ = delta_tx
                                            .send(crate::openhuman::inference::provider::ProviderDelta::ToolCallStart {
                                                call_id: id.clone(),
                                                tool_name: name.clone(),
                                            })
                                            .await;
                                        entry.emitted_start = true;
                                        if !entry.arguments.is_empty() {
                                            log::debug!(
                                                "[stream] {} tool_call[{}] flushing buffered args ({} chars)",
                                                self.name,
                                                idx,
                                                entry.arguments.len(),
                                            );
                                            let buffered = entry.arguments.clone();
                                            let _ = delta_tx
                                                .send(crate::openhuman::inference::provider::ProviderDelta::ToolCallArgsDelta {
                                                    call_id: id.clone(),
                                                    delta: buffered,
                                                })
                                                .await;
                                            entry.emitted_chars = entry.arguments.len();
                                        }
                                    }
                                } else if entry.arguments.len() > entry.emitted_chars {
                                    if let Some(ref id) = entry.id {
                                        let fresh =
                                            entry.arguments[entry.emitted_chars..].to_string();
                                        let _ = delta_tx
                                            .send(crate::openhuman::inference::provider::ProviderDelta::ToolCallArgsDelta {
                                                call_id: id.clone(),
                                                delta: fresh,
                                            })
                                            .await;
                                        entry.emitted_chars = entry.arguments.len();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if degenerate_repeat {
            text_accum.push_str(
                "\n\n[Output stopped: detected repeated/looping generation (model degeneration).]",
            );
        }

        let tool_call_count = tool_accum.len();
        log::info!(
            "[stream] {} aggregated text_chars={} thinking_chars={} tool_calls={}",
            self.name,
            text_accum.chars().count(),
            thinking_accum.chars().count(),
            tool_call_count,
        );

        let tool_calls_for_api: Vec<ToolCall> = tool_accum
            .into_values()
            .map(|c| ToolCall {
                id: c.id,
                kind: Some("function".to_string()),
                function: Some(super::compatible_types::Function {
                    name: c.name,
                    arguments: if c.arguments.is_empty() {
                        None
                    } else {
                        Some(
                            serde_json::from_str(&c.arguments)
                                .unwrap_or(serde_json::Value::String(c.arguments)),
                        )
                    },
                }),
                // Carry Gemini's thought_signature through to parse_native_response
                // so it lands on the harness ToolCall (TAURI-RUST-4PK).
                extra_content: c.extra_content,
            })
            .collect();

        let api_resp = ApiChatResponse {
            choices: vec![Choice {
                message: ResponseMessage {
                    content: if text_accum.is_empty() {
                        None
                    } else {
                        Some(text_accum)
                    },
                    reasoning_content: if thinking_accum.is_empty() {
                        None
                    } else {
                        Some(thinking_accum)
                    },
                    tool_calls: if tool_calls_for_api.is_empty() {
                        None
                    } else {
                        Some(tool_calls_for_api)
                    },
                    function_call: None,
                },
            }],
            usage: last_usage,
            openhuman: last_openhuman,
        };

        if std::env::var("OPENHUMAN_PROMPT_DUMP_DIR").is_ok() {
            let msg = &api_resp.choices[0].message;
            let aggregated = serde_json::json!({
                "content": msg.content,
                "reasoning_content": msg.reasoning_content,
                "tool_calls": msg.tool_calls.as_ref().map(|calls| {
                    calls.iter().map(|c| serde_json::json!({
                        "id": c.id,
                        "type": c.kind,
                        "function": c.function.as_ref().map(|f| serde_json::json!({
                            "name": f.name,
                            "arguments": f.arguments,
                        })),
                    })).collect::<Vec<_>>()
                }),
                "usage": api_resp.usage.as_ref().map(|u| serde_json::json!({
                    "prompt_tokens": u.prompt_tokens,
                    "completion_tokens": u.completion_tokens,
                    "total_tokens": u.total_tokens,
                    "prompt_cached_tokens": u.prompt_tokens_details
                        .as_ref().map(|d| d.cached_tokens),
                })),
                "openhuman": api_resp.openhuman.as_ref().map(|m| serde_json::json!({
                    "usage": m.usage.as_ref().map(|u| serde_json::json!({
                        "input_tokens": u.input_tokens,
                        "output_tokens": u.output_tokens,
                        "cached_input_tokens": u.cached_input_tokens,
                    })),
                    "billing": m.billing.as_ref().map(|b| serde_json::json!({
                        "charged_amount_usd": b.charged_amount_usd,
                    })),
                })),
            });
            if let Ok(bytes) = serde_json::to_vec(&aggregated) {
                dump_response_if_enabled(&self.name, &native_request.model, dump_seq, &bytes);
            }
        }

        Self::parse_native_response(api_resp, &self.name)
    }
}
