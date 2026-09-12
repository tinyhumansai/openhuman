use super::*;

#[test]
fn non_codex_token_expired_does_not_classify_as_codex_oauth() {
    // A generic provider error containing "token_expired" (without the Codex
    // sentinel prefix) must not route to the Codex reconnect flow. (#5869)
    let err = "openai error 401: {\"error\":{\"code\":\"token_expired\",\
               \"message\":\"Please try signing in again.\"}}";
    let classified = classify_inference_error(err);
    assert_ne!(
        classified.provider.as_deref(),
        Some("openai_codex"),
        "generic token_expired error must not route to the Codex reconnect flow: {}",
        classified.message
    );
}
