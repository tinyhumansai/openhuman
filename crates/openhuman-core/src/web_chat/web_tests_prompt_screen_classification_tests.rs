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
