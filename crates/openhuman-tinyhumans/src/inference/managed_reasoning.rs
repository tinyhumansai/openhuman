//! Managed-backend wire extensions: the `reasoning` object and the `thread_id`
//! field, derived from the provider-neutral request.

use serde_json::Value;
use tinyinference_llm::model::ModelRequest;

use openhuman_embed::__host::inference_registry::REASONING_OFF_METADATA_KEY;

/// Translate the request's reasoning choice into the managed backend's wire
/// field: the OpenRouter-style `reasoning` object, which the backend forwards
/// upstream.
///
/// Sources, highest first: a `reasoning` object the caller already put in
/// provider options (left alone); the `without_reasoning` hint (core, `REASONING_OFF_METADATA_KEY`), which marks
/// one helper call specifically; the request's provider-neutral
/// `ModelRequest::reasoning` (the user's thinking level, from the harness
/// `RunPolicy::default_reasoning`). The
/// neutral field is consumed here, so the OpenAI-compatible transport does not
/// also emit a top-level `reasoning_effort` for the same choice.
pub(super) fn apply_reasoning_hint(mut request: ModelRequest) -> ModelRequest {
    if request.provider_options.get("reasoning").is_some() {
        return request;
    }
    let wants_off = request
        .metadata
        .get(REASONING_OFF_METADATA_KEY)
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let neutral = request.reasoning.take();
    let wire = if wants_off {
        Some(serde_json::json!({ "enabled": false }))
    } else {
        neutral.as_ref().and_then(managed_reasoning_wire)
    };
    let Some(wire) = wire else {
        return request;
    };
    let mut options = request.provider_options.clone();
    if !options.is_object() {
        options = Value::Object(serde_json::Map::new());
    }
    if let Some(map) = options.as_object_mut() {
        map.insert("reasoning".to_string(), wire.clone());
    }
    log::debug!("[inference][managed] reasoning for this call: {wire}");
    request.with_provider_options(options)
}

/// The OpenRouter `reasoning` object for a provider-neutral config: `none`
/// disables reasoning, an explicit budget becomes `max_tokens`, and any other
/// effort is sent by name. An empty config sends nothing.
fn managed_reasoning_wire(reasoning: &tinyinference_llm::model::ReasoningConfig) -> Option<Value> {
    use tinyinference_llm::model::ReasoningEffort;
    if reasoning.effort == Some(ReasoningEffort::None) {
        return Some(serde_json::json!({ "enabled": false }));
    }
    if let Some(budget) = reasoning.budget_tokens {
        return Some(serde_json::json!({ "max_tokens": budget }));
    }
    reasoning
        .effort
        .map(|effort| serde_json::json!({ "effort": effort.as_str() }))
}

/// Inject this managed model's explicitly owned thread into provider options.
/// This backend-only wire extension is never inferred from ambient state.
pub(super) fn with_thread_id(request: ModelRequest, thread_id: Option<&str>) -> ModelRequest {
    let Some(thread_id) = thread_id else {
        return request;
    };
    let mut options = request.provider_options.clone();
    if !options.is_object() {
        options = Value::Object(serde_json::Map::new());
    }
    if let Some(map) = options.as_object_mut() {
        map.insert(
            "thread_id".to_string(),
            Value::String(thread_id.to_string()),
        );
    }
    request.with_provider_options(options)
}

#[cfg(test)]
#[path = "managed_reasoning_tests.rs"]
mod tests;
