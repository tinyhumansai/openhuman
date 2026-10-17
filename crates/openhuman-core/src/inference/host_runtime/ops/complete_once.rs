//! One stateless model call on an explicit OpenAI-compatible endpoint: no
//! session, no tools, no prompt guard.
//!
//! [`agent_chat_simple`](super::agent_chat_simple) is the closest neighbour,
//! and the differences are the point of this module:
//!
//! - **No prompt guard.** `enforce_user_prompt_or_reject` exists to stop a
//!   user steering an agent that holds tools. A completion holds none, and its
//!   callers (code reviewers, classifiers, extractors) routinely feed it
//!   adversarial text *as data* — a guard would reject exactly the inputs they
//!   exist to read.
//! - **The caller's request, not a message string.** System/user roles, images,
//!   `response_format`, `max_tokens` and pass-through `provider_options` all
//!   reach the wire, and the whole [`ModelResponse`] (finish reason, usage, raw
//!   provider body) comes back.
//! - **Tools are refused, not ignored.** A request that declares tools is an
//!   error: a host that wants a tool loop wants an agent turn, and silently
//!   dropping the declarations would hide that mistake.
//! - **No provider resolution.** The endpoint is the caller's, and the model is
//!   built directly on it rather than through the config-driven provider
//!   factory, so there is no managed-backend detour, no role pin and no
//!   fallback: the model a caller pays for is the one it named.

use tinyinference_llm::model::{ChatModel, ModelRequest, ModelResponse};
use tinyinference_llm::providers::openai::OpenAiModel;

/// Where a [`complete_once`] call goes.
#[derive(Clone)]
pub struct CompletionEndpoint {
    /// OpenAI-compatible base URL; `/chat/completions` is appended.
    pub base_url: String,
    /// Bearer presented to `base_url`.
    pub api_key: String,
    /// Extra request headers (gateway attribution such as OpenRouter's
    /// `HTTP-Referer` / `X-Title`).
    pub headers: Vec<(String, String)>,
}

impl std::fmt::Debug for CompletionEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompletionEndpoint")
            .field(
                "base_url",
                &crate::inference::provider::factory::redact_endpoint(&self.base_url),
            )
            .field("api_key", &"<redacted>")
            .field("headers", &self.headers.len())
            .finish()
    }
}

/// Run `request` once against `endpoint` and return the provider's response.
///
/// `request.model` is required and sent verbatim. Errors are rendered strings,
/// the same contract as the other `ops` functions, so the facade maps them in
/// one place.
///
/// `budget` is the caller's explicit spend ledger; every physical call the
/// request makes is charged against it. `None` runs unmetered.
pub async fn complete_once(
    endpoint: &CompletionEndpoint,
    request: ModelRequest,
    budget: Option<&crate::agent::tinyagents::budget::ModelBudget>,
) -> Result<ModelResponse, String> {
    if !request.tools.is_empty() {
        return Err(
            "complete_once: tools are not supported on a stateless completion; use an agent turn"
                .to_string(),
        );
    }
    // `provider_options` is forwarded untouched, so a `tools` key there would
    // reach the provider even though `request.tools` is empty above. The typed
    // request fields own these keys.
    if let Some(key) = reserved_provider_option(&request.provider_options) {
        return Err(format!(
            "complete_once: provider_options may not set `{key}`; use the typed request fields"
        ));
    }
    // Blank is refused, but the id itself goes out exactly as the caller named it.
    let model_id = request
        .model
        .as_deref()
        .filter(|model| !model.trim().is_empty())
        .ok_or_else(|| "complete_once: request.model is required".to_string())?
        .to_string();

    let mut model = OpenAiModel::new(endpoint.api_key.clone())
        .with_base_url(endpoint.base_url.clone())
        .with_model(model_id.clone());
    for (name, value) in &endpoint.headers {
        model = model.with_header(name.clone(), value.clone());
    }

    tracing::debug!(
        model = %model_id,
        messages = request.messages.len(),
        response_format = request.response_format.is_some(),
        max_tokens = ?request.max_tokens,
        "[inference] complete_once invoking chat model"
    );

    let model: std::sync::Arc<dyn ChatModel<()>> = std::sync::Arc::new(model);
    let model = match budget {
        Some(budget) => budget.wrap(model),
        None => model,
    };
    model
        .invoke(&(), request)
        .await
        .map_err(|e| format!("complete_once: {e}"))
}

/// Body keys that `provider_options` may not carry: the tool surface (which
/// this op refuses) and the fields the typed request already sets.
const RESERVED_PROVIDER_OPTIONS: &[&str] = &[
    "model",
    "messages",
    "tools",
    "tool_choice",
    "functions",
    "function_call",
    "response_format",
    "stream",
];

fn reserved_provider_option(options: &serde_json::Value) -> Option<&'static str> {
    let object = options.as_object()?;
    RESERVED_PROVIDER_OPTIONS
        .iter()
        .copied()
        .find(|key| object.contains_key(*key))
}

#[cfg(test)]
#[path = "complete_once_tests.rs"]
mod tests;
