//! The `LlmProvider` capability, backed by OpenHuman's inference stack.
//!
//! What an `agent` node falls back to when no agent runner is installed, and
//! what a raw completion node uses directly.

#![allow(unused_imports)]

use std::sync::Arc;

use serde_json::{json, Value};

use super::prompt::build_completion_messages;
use super::*;
use crate::config::Config;
use crate::inference::provider::create_chat_model_with_model_id_in;
use crate::modules::tinyflows as module;
use tinyinference_llm::model::ModelRequest;

pub(crate) fn resolve_host_model(node_model: Option<&str>, provider_model: String) -> String {
    match node_model.filter(|model| {
        crate::inference::provider_model::is_raw_model(
            crate::inference::provider::factory::PROVIDER_OPENHUMAN,
            model,
        )
    }) {
        Some(model) => model.to_owned(),
        None => provider_model,
    }
}

/// [`LlmProvider`] adapter over OpenHuman's inference stack
/// (`crates/openhuman-core/src/inference/provider/`).
///
/// The `agent` node is single-completion in tinyflows 0.2 (no tool-calling
/// loop, no sub-ports), so `complete` performs exactly one `provider.chat`
/// call and returns its result — no agent loop is driven here.
///
/// **Structured output**: when the node requested it (an
/// `output_parser.schema` or `response_format: "json"` in the config), the
/// completion text is parsed as JSON and the **parsed object** is returned as
/// the response value; otherwise the `{text: "..."}` shape is returned. Either
/// way the tinyflows `agent` node wraps this in its stable output **envelope**
/// `{ json, text, raw }`, so a downstream node binds `=item.json.<field>` for
/// structured output or `=item.text` for prose (or
/// `=nodes.<agent_id>.item.json.<field>` across nodes) — the parsed-vs-`{text}`
/// shape is no longer visible to consumers. A completion that doesn't parse
/// still lets the agent node's `output_parser` sub-port coerce it via the
/// schema auto-fix path before enveloping.
pub struct OpenHumanLlm {
    pub ctx: crate::core::runtime::DispatchCtx,
    pub config: Arc<Config>,
}

impl OpenHumanLlm {
    pub(crate) async fn complete(&self, request: Value, conn: Option<&str>) -> Result<Value> {
        self.complete_with_format_request(request.clone(), request, conn)
            .await
    }

    pub(crate) async fn complete_with_format_request(
        &self,
        request: Value,
        format_request: Value,
        conn: Option<&str>,
    ) -> Result<Value> {
        if let Some(c) = conn {
            // B1 does not resolve `connection_ref` to a specific BYOK account —
            // `create_chat_provider` picks the configured provider for `role`.
            tracing::debug!(target: "flows", conn = %c, "[flows] llm conn (not resolved in B1)");
        }

        let role = request
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("summarization");

        // Per-node model selection: an `agent` node may pin a **managed tier**
        // (`config.model = "hint:reasoning"` / `"hint:chat"`, or a `hint:*` alias).
        // Map that tier back to the workload role whose provider serves it so
        // the completion routes to that tier on the managed backend (or the
        // role's BYOK model) instead of the node's default `role`. Unknown /
        // absent model strings leave the role untouched. `config.model` is
        // trusted node config, never model output.
        let node_model = request
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let role = match node_model {
            Some(model) => {
                let mapped = crate::inference::provider_model::role_for_provider_model(
                    crate::inference::provider::factory::PROVIDER_OPENHUMAN,
                    model,
                );
                tracing::debug!(
                    target: "flows",
                    node_model = model,
                    mapped_role = mapped,
                    "[flows] llm.complete: node pinned a model tier — routing by mapped role"
                );
                mapped
            }
            None => role,
        };
        let temperature = request
            .get("temperature")
            .and_then(Value::as_f64)
            .unwrap_or(0.7);
        let max_tokens = request
            .get("max_tokens")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok());

        let portability = module::portability_proxy(&self.config)
            .await
            .map_err(EngineError::Capability)?;
        let prepared: tinyflows_bus::portability::PreparedLlmComplete = portability
            .call(
                tinyflows_bus::portability::methods::PREPARE_LLM_COMPLETE,
                &(tinyflows_bus::portability::PrepareLlmComplete {
                    request: request.clone(),
                },),
            )
            .await
            .map_err(|error| EngineError::Capability(error.to_string()))?;
        let node_model = prepared.node_model.as_deref();
        let structured = prepared.structured_output;
        let messages = build_completion_messages(&prepared.messages);

        tracing::debug!(
            target: "flows",
            role,
            message_count = messages.len(),
            structured,
            "[flows] llm.complete: dispatching agent-node completion"
        );

        let (chat_model, model) =
            create_chat_model_with_model_id_in(&self.ctx, role, &self.config, temperature)
                .await
                .map_err(|e| EngineError::Capability(e.to_string()))?;
        // `create_chat_provider` handed back the role's default model. If the node
        // pinned a raw/BYOK id, forward it verbatim instead (issue #4598).
        let model = resolve_host_model(node_model, model);

        let mut model_request = ModelRequest::new(
            messages
                .iter()
                .map(crate::agent::tinyagents::chat_message_to_message)
                .collect(),
        )
        .with_model(model.clone())
        .with_temperature(temperature);
        if let Some(max_tokens) = max_tokens {
            model_request = model_request.with_max_tokens(max_tokens);
        }
        let response = chat_model
            .invoke(&(), model_request)
            .await
            .map_err(|e| EngineError::Capability(e.to_string()))?;

        // Structured mode: surface the parsed object itself so downstream
        // `=item.<field>` / `=nodes.<id>.item.<field>` bindings work. The
        // agent node's output_parser sub-port then validates it against the
        // configured schema (and auto-fixes when it doesn't parse here).
        let host_response = model_response_to_completion_value(&response);
        let formatted: Value = portability
            .call(
                tinyflows_bus::portability::methods::FORMAT_LLM_COMPLETE,
                &(tinyflows_bus::portability::FormatLlmComplete {
                    request: format_request,
                    response: host_response,
                },),
            )
            .await
            .map_err(|error| EngineError::Capability(error.to_string()))?;
        if structured && !formatted.is_object() && !formatted.is_array() {
            tracing::warn!(
                target: "flows",
                "[flows] llm.complete: structured output requested but module did not extract JSON"
            );
        }
        Ok(formatted)
    }
}
