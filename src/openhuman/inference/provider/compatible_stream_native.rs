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

        // Captured at request send so the empty-2xx-stream diagnostic
        // below can report elapsed_ms — a fast empty stream points at a
        // backend reject, a slow one at an upstream stall / timeout.
        let stream_started_at = std::time::Instant::now();

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
            } else if super::super::is_byo_provider_auth_failure_http(
                self.name.as_str(),
                status,
                &body,
            ) {
                super::super::log_byo_provider_auth_failure(
                    "streaming_chat",
                    self.name.as_str(),
                    Some(native_request.model.as_str()),
                    status,
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
            let body_bytes_received = response_bytes.len();
            dump_response_if_enabled(&self.name, &native_request.model, dump_seq, &response_bytes);
            let api_resp: ApiChatResponse = serde_json::from_slice(&response_bytes)
                .map_err(|err| anyhow::anyhow!("{} response parse error: {err}", self.name))?;

            // Mirror the SSE-branch empty-2xx-stream diagnostic (#3335 /
            // #3386) on the buffered JSON path. The same upstream
            // collapse to `AgentError::EmptyProviderResponse` is
            // reachable here when a managed backend returns 200 with a
            // content-less JSON payload (credit exhaustion served as
            // JSON instead of SSE, or an upstream stall flushed as an
            // empty completion). Without this sibling guard the warn
            // would only fire on the SSE branch and the buffered case
            // would silently miss the very signal we're trying to
            // capture.
            let buffered_is_empty = api_resp
                .choices
                .first()
                .map(|c| {
                    let m = &c.message;
                    let content_empty = m.content.as_deref().is_none_or(str::is_empty);
                    let reasoning_empty = m.reasoning_content.as_deref().is_none_or(str::is_empty);
                    let tool_calls_empty = m.tool_calls.as_ref().is_none_or(|t| t.is_empty());
                    let function_call_empty = m.function_call.is_none();
                    content_empty && reasoning_empty && tool_calls_empty && function_call_empty
                })
                .unwrap_or(true);
            if buffered_is_empty {
                let elapsed_ms = stream_started_at.elapsed().as_millis() as u64;
                log::warn!(
                    "[stream] {} empty 2xx buffered JSON — model={} elapsed_ms={} body_bytes={} has_usage={} has_openhuman_meta={}",
                    self.name,
                    native_request.model,
                    elapsed_ms,
                    body_bytes_received,
                    api_resp.usage.is_some(),
                    api_resp.openhuman.is_some(),
                );
            }
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
        // Forensic counters for the empty-2xx-stream diagnostic below
        // (issue #3335 / #3386). Both are append-only, never read by
        // request path logic — strictly observability.
        //
        // `body_bytes_received` is the count of body bytes yielded by
        // `bytes_stream()`. This crate builds reqwest without the
        // `gzip` / `brotli` / `zstd` / `deflate` features (see
        // root Cargo.toml), so no Content-Encoding decompression happens
        // and the count matches what's on the wire. The neutral name
        // (rather than `raw_bytes` / `decoded_bytes`) sidesteps the
        // ambiguity an operator would otherwise hit reading the log.
        let mut sse_chunks_parsed: usize = 0;
        let mut body_bytes_received: usize = 0;

        'stream: while let Some(item) = bytes_stream.next().await {
            let bytes = item?;
            body_bytes_received += bytes.len();
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            while let Some(sep_idx) = buffer.find("\n\n") {
                let event = buffer[..sep_idx].to_string();
                buffer.drain(..sep_idx + 2);

                // In-band SSE error frame. The response status flushed 200
                // before the upstream call, so the managed backend delivers
                // post-flush errors — including instant 4xx/429/413 whose typed
                // `errorCode` (#870) is now stamped on the frame, see
                // backend `routes/inference.ts` — as `event: error\ndata: {…}`.
                // Surface it as a streaming-envelope error string so the
                // `errorCode` classifier + Sentry golden rule run downstream,
                // instead of skipping it as an unparseable chunk (which would
                // aggregate to empty and surface as "empty response").
                if let Some(message) = sse_error_frame_bail_message(
                    self.name.as_str(),
                    Some(native_request.model.as_str()),
                    &event,
                ) {
                    anyhow::bail!(message);
                }

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
                        Ok(v) => {
                            sse_chunks_parsed += 1;
                            v
                        }
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

                                // Only the FIRST delta for a tool-call index carries
                                // the real id; argument-continuation deltas repeat the
                                // index with an EMPTY id (`""`) on some providers
                                // (DashScope/GMI) and omit it entirely on others
                                // (DeepSeek). Guard against the empty string so a
                                // continuation delta can't clobber the resolved id down
                                // to `""` — which later trips the upstream tool-message
                                // ordering check (empty `tool_call_id` → 400).
                                if let Some(id) = tc.id.as_ref() {
                                    if !id.is_empty() {
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

        // Issue #3335 / #3386 forensic signal. The streaming chat call
        // completed with HTTP 2xx but delivered zero visible text, zero
        // thinking, and zero tool calls. This is the upstream shape that
        // collapses to `AgentError::EmptyProviderResponse` and gets
        // rendered to the user as "The model returned an empty
        // response" with the wrong remediation. Most likely causes:
        //   (a) backend closed the SSE cleanly under credit exhaustion
        //       (no `[stream] streaming API error` breadcrumb fires
        //       because status was 200) — common on the OpenHuman
        //       managed route under #3386,
        //   (b) backend's upstream LLM provider stalled / timed out
        //       and the backend forwarded an empty stream instead of
        //       propagating the upstream error,
        //   (c) a genuine degenerate model output (rare on hosted
        //       reasoning models; more common on community quants).
        // Logged at warn so it lands in Sentry breadcrumbs even after
        // `AgentError::skips_sentry()` (PR #2790) silences the parent
        // event. Correlate by elapsed_ms (fast == reject, slow ==
        // stall), sse_chunks (0 == no SSE at all, >0 == backend
        // streamed metadata-only chunks), has_usage (the upstream
        // counted tokens but delivered no content), and
        // has_openhuman_meta (managed backend reported routing info).
        if text_accum.is_empty() && thinking_accum.is_empty() && tool_call_count == 0 {
            let elapsed_ms = stream_started_at.elapsed().as_millis() as u64;
            log::warn!(
                "[stream] {} empty 2xx stream — model={} elapsed_ms={} sse_chunks={} body_bytes={} has_usage={} has_openhuman_meta={}",
                self.name,
                native_request.model,
                elapsed_ms,
                sse_chunks_parsed,
                body_bytes_received,
                last_usage.is_some(),
                last_openhuman.is_some(),
            );
        }

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

/// Extract the `data:` payload of an SSE `event: error` frame, or `None` when
/// the event block is not an error frame.
///
/// The managed backend delivers post-flush stream errors as a two-line SSE
/// block — `event: error` followed by `data: {"error":{…,"errorCode":…}}`
/// (backend `routes/inference.ts`). The normal chunk loop can't parse that
/// `data:` line as a `StreamChunkResponse`, so without this detection the frame
/// is silently skipped and the turn aggregates to an empty response. We key on
/// the `event: error` field rather than sniffing the data so a normal chunk
/// that merely mentions "error" is never misclassified.
fn sse_error_frame_payload(event: &str) -> Option<String> {
    let mut is_error_frame = false;
    let mut data: Option<String> = None;
    for line in event.lines() {
        let line = line.trim();
        if let Some(event_type) = line.strip_prefix("event:") {
            if event_type.trim() == "error" {
                is_error_frame = true;
            }
        } else if let Some(payload) = line.strip_prefix("data:") {
            data = Some(payload.trim().to_string());
        }
    }
    is_error_frame.then(|| data.unwrap_or_default())
}

/// Build the bail message for an in-band SSE `event: error` frame, or `None`
/// when the event block is a normal chunk / metadata event (the caller then
/// continues parsing it as a chunk).
///
/// Mirrors the non-2xx HTTP path: produces a `<provider> streaming API error:
/// <sanitized body>` envelope so the downstream `errorCode` classifier and the
/// Sentry golden rule run on it. When the frame carries a backend-owned
/// `errorCode`, it is logged (not re-reported) here, matching the HTTP path's
/// [`is_backend_error_code_owned`] branch; a malformed `BAD_REQUEST` is excluded
/// by that helper and still pages downstream. There is no HTTP status for an
/// in-band frame, so the log records the `200` that was actually sent.
fn sse_error_frame_bail_message(
    provider: &str,
    model: Option<&str>,
    event: &str,
) -> Option<String> {
    let payload = sse_error_frame_payload(event)?;
    let sanitized = super::super::sanitize_api_error(&payload);
    let message = format!("{provider} streaming API error: {sanitized}");
    if super::super::is_backend_error_code_owned(provider, &payload) {
        super::super::log_backend_error_code_owned(
            "streaming_chat",
            provider,
            model,
            reqwest::StatusCode::OK,
            &payload,
        );
    }
    Some(message)
}

#[cfg(test)]
mod sse_error_frame_tests {
    use super::{sse_error_frame_bail_message, sse_error_frame_payload};
    use crate::openhuman::inference::provider::openhuman_backend::PROVIDER_LABEL;

    #[test]
    fn extracts_payload_from_error_frame() {
        let event = "event: error\ndata: {\"error\":{\"message\":\"boom\",\"type\":\"stream_error\",\"errorCode\":\"BAD_REQUEST\"}}";
        let payload = sse_error_frame_payload(event).expect("error frame detected");
        assert!(payload.contains("\"errorCode\":\"BAD_REQUEST\""));
        assert!(payload.contains("\"type\":\"stream_error\""));
    }

    #[test]
    fn ignores_normal_data_chunk() {
        let event =
            "data: {\"choices\":[{\"delta\":{\"content\":\"an error occurred in the story\"}}]}";
        assert!(sse_error_frame_payload(event).is_none());
    }

    #[test]
    fn ignores_metadata_event() {
        let event = "event: openhuman-metadata\ndata: {\"openhuman\":{}}";
        assert!(sse_error_frame_payload(event).is_none());
    }

    #[test]
    fn bail_message_wraps_error_frame_in_streaming_envelope() {
        // A managed-backend error frame yields a streaming-envelope string the
        // downstream classifier recognises, preserving the `errorCode`.
        let event = "event: error\ndata: {\"error\":{\"message\":\"slow down\",\"type\":\"stream_error\",\"errorCode\":\"RATE_LIMITED\"}}";
        let message = sse_error_frame_bail_message(PROVIDER_LABEL, Some("reasoning-v1"), event)
            .expect("bail message built");
        assert!(message.contains("streaming API error"));
        assert!(message.contains("\"errorCode\":\"RATE_LIMITED\""));
    }

    #[test]
    fn bail_message_handles_frame_without_error_code() {
        // An untyped mid-stream drop (no `errorCode`) still produces an
        // envelope; it is simply not backend-owned, so downstream Sentry gating
        // applies as before.
        let event =
            "event: error\ndata: {\"error\":{\"message\":\"socket hang up\",\"type\":\"stream_error\"}}";
        let message =
            sse_error_frame_bail_message(PROVIDER_LABEL, None, event).expect("bail message built");
        assert!(message.contains("socket hang up"));
    }

    #[test]
    fn bail_message_is_none_for_normal_chunk() {
        let event = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}";
        assert!(sse_error_frame_bail_message(PROVIDER_LABEL, None, event).is_none());
    }

    #[test]
    fn error_frame_without_data_yields_empty_payload() {
        // Defensive: an `event: error` with no `data:` line still resolves to a
        // (empty) payload so the caller bails rather than skipping it.
        let event = "event: error";
        assert_eq!(sse_error_frame_payload(event).as_deref(), Some(""));
    }
}
