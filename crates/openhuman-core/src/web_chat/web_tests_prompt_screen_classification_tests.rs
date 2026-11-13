use super::*;

/// TS-E2E-014: a turn the prompt-injection screen held never reached the
/// model, so it must not be reported as a provider failure ("Something went
/// wrong", source provider). Each of the host's refusal texts classifies to
/// the dedicated `prompt_screened` row.
#[test]
fn prompt_screen_refusals_are_not_reported_as_provider_failures() {
    for raw in [
        "Prompt flagged for security review and was not processed.",
        "Prompt blocked by security policy.",
        "Content from 'user' was rejected by the prompt-injection screen (screen.unavailable). \
         It has not been added to the conversation.",
    ] {
        let classified = classify_inference_error(raw);
        assert_eq!(classified.error_type, "prompt_screened", "{raw}");
        assert_eq!(classified.source, "security", "{raw}");
        assert_eq!(classified.copy_key, "chat_error.prompt_screened", "{raw}");
        assert!(classified.retryable, "{raw}");
        assert!(classified.provider.is_none(), "{raw}");
    }
}

/// A provider's own error body with the same words is the provider's refusal:
/// it keeps the provider and is not reported as OpenHuman's screen.
#[test]
fn a_provider_error_quoting_the_screen_text_stays_a_provider_error() {
    let classified = classify_inference_error(
        "custom_openai API error (400 Bad Request): Prompt blocked by security policy.",
    );
    assert_ne!(classified.error_type, "prompt_screened");
    assert_ne!(classified.source, "security");
    assert!(classified.provider.is_some(), "{classified:?}");
}
