//! Classifier for **chat-template rejections** from local serving runtimes.
//!
//! A local runtime (LM Studio, llama.cpp, Ollama) renders every request
//! through the model's own Jinja chat template, baked into the GGUF. When
//! the template cannot render the message list it aborts with a `400`
//! *before* the model is ever called, and the body describes a template
//! failure — not a bad model id, not a bad sampling parameter:
//!
//! ```text
//! lmstudio returned: Engine protocol predict request returned 400:
//! {"error":{"code":400,"message":"Unable to generate parser for this
//! template. Automatic parser generation failed: While executing
//! CallExpression at line 79, column 24 in source: ...multi_step_tool %}
//! {{- raise_exception('No user query found in messages.') }}...
//! Error: Jinja Exception: No user query found in messages.",
//! "type":"invalid_request_error"}}
//! ```
//!
//! The canonical instance (tinyhumansai/openhuman#5291) is Qwen 3's
//! required-user-query guard firing against the prompt-guided tool loop's
//! message shape, on a model that reports no native tool calling. The
//! harness-side fix is to guarantee a resolvable user turn
//! (`tinyagents::harness::tool::ensure_resolvable_user_turn`); this
//! classifier exists so the class is *named accurately* while it happens
//! — with any template, any model.
//!
//! ## Why it needs its own arm
//!
//! Nothing in the raw body matches
//! [`super::config_rejection::is_provider_config_rejection_message`], but
//! the retry aggregate that wraps two failed attempts does (`"may not be
//! available on your provider"`), so the user was shown *"Your AI provider
//! rejected the request's model or temperature setting. Check your model
//! and routing in Settings → LLM."* That is wrong in a way that costs the
//! user real time: the model is fine, the temperature is fine, and every
//! remediation it suggests is a dead end. Classified ahead of the
//! config-rejection arm, the template failure keeps its own identity even
//! when it reaches the classifier inside an aggregate.
//!
//! Deliberately NOT added to
//! [`crate::core::observability::expected_error_kind`]: unlike a user
//! picking an unavailable model, a template that rejects our own message
//! shape is a defect on our side, and it should stay visible until the
//! harness normalization has shipped everywhere.

/// Returns true if a provider error body indicates the model's **chat
/// template** rejected the request — a template render/parse failure, as
/// opposed to a rejected model id, sampling parameter, or credential.
///
/// Case-insensitive substring match, anchored on phrases emitted by the
/// template engine itself (`raise_exception` text, the Jinja exception
/// marker) and by LM Studio's template-parser generation, so it holds
/// across runtimes that embed the same Jinja templates. Keep the list
/// tight: a false positive would relabel an unrelated provider error as a
/// template problem and send the user chasing a template they cannot see.
pub fn is_chat_template_rejection_message(body: &str) -> bool {
    const PHRASES: &[&str] = &[
        // Qwen 3-family required-user-query guard — the #5291 repro. Raised
        // by the template when it cannot locate a user turn to answer.
        "no user query found in messages",
        // LM Studio wraps a template it cannot compile into a tool-call
        // parser with this prefix; the inner cause is always a template
        // render failure.
        "unable to generate parser for this template",
        "automatic parser generation failed",
        // Generic escape hatch for any other template that raises: the
        // engine tags every one of these with the Jinja exception marker.
        // Covers the sibling guards in Llama-3 / Mistral / ChatML templates
        // (alternating-role requirements, "conversation roles must
        // alternate", tool-response ordering) without enumerating each.
        "jinja exception",
    ];

    let lower = body.to_ascii_lowercase();
    PHRASES.iter().any(|phrase| lower.contains(phrase))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The verbatim body from the #5291 user log, wrapper and all.
    const LMSTUDIO_5291_BODY: &str = "lmstudio returned: Engine protocol predict request \
         returned 400: {\"error\":{\"code\":400,\"message\":\"Unable to generate parser for \
         this template. Automatic parser generation failed: While executing CallExpression \
         at line 79, column 24 in source: ...multi_step_tool %}  {{- raise_exception('No \
         user query found in messages.') }}...Error: Jinja Exception: No user query found \
         in messages.\",\"type\":\"invalid_request_error\"}}";

    #[test]
    fn classifies_the_lmstudio_template_body() {
        assert!(is_chat_template_rejection_message(LMSTUDIO_5291_BODY));
    }

    #[test]
    fn classifies_inside_a_retry_aggregate() {
        // Two attempts fail and the aggregate wraps both; the anchor must
        // survive so the template arm still wins over the config-rejection
        // arm the aggregate's own wording would otherwise trip.
        let aggregate = format!(
            "The model `qwen/qwen3.5-9b` may not be available on your provider. \
             Configure a fallback chain via `reliability.model_fallbacks` in your \
             OpenHuman config.\n\nAll providers/models failed. Attempts:\n\
             provider=lmstudio model=qwen/qwen3.5-9b attempt 1/2: {LMSTUDIO_5291_BODY}\n\
             provider=lmstudio model=qwen/qwen3.5-9b attempt 2/2: {LMSTUDIO_5291_BODY}"
        );
        assert!(is_chat_template_rejection_message(&aggregate));
    }

    #[test]
    fn detection_is_case_insensitive() {
        assert!(is_chat_template_rejection_message(
            "Error: JINJA EXCEPTION: No User Query Found In Messages."
        ));
    }

    #[test]
    fn does_not_classify_unrelated_provider_errors() {
        for body in [
            "openai API error (400): invalid temperature: only 1 is allowed for this model",
            "The model `gpt-5.5` does not exist or you do not have access to it.",
            "lmstudio returned: model 'qwen3.5-9b' does not support tools",
            "openrouter API error (429): rate limited",
            // A prose mention of templates is not a template rejection.
            "Failed to render the prompt template file on disk",
        ] {
            assert!(
                !is_chat_template_rejection_message(body),
                "{body:?} must not be classified as a chat-template rejection"
            );
        }
    }
}
