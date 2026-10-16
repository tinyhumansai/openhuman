#[test]
fn tts_unauthorized_flattens_to_session_expiry_not_hard_error() {
    // TAURI-RUST-8X1: a lapsed-session 401 on the TTS endpoint
    // (`POST /openai/v1/audio/speech`) used to be flattened with
    // `e.to_string()`, producing the raw "backend rejected session token …"
    // Display string that matched none of the session-expiry classifiers and
    // leaked to Sentry as a hard error. `synthesize_reply` now flattens the
    // typed `BackendApiError::Unauthorized` via `crate::client::flatten_authed_error`
    // (the #3384 team/billing pattern), so it carries the SESSION_EXPIRED
    // sentinel and is recognised + demoted by the JSON-RPC dispatcher.
    //
    // This test couples the exact TTS endpoint's typed 401 to the live
    // classifier: build the typed error → flatten → classify. If either the
    // sentinel mapping or the classifier drifts, this fails instead of
    // silently re-leaking the TTS 401.
    let flat = crate::client::flatten_authed_error(anyhow::Error::new(
        crate::client::HostedApiError::Unauthorized {
            method: "POST".to_string(),
            path: "/openai/v1/audio/speech".to_string(),
        },
    ));

    assert!(
        flat.contains("SESSION_EXPIRED"),
        "flattened TTS 401 must carry the sentinel, got: {flat}"
    );
    assert!(
        flat.contains("/openai/v1/audio/speech"),
        "path preserved for logs: {flat}"
    );
    assert!(
        openhuman_embed::__host::core::observability::is_session_expired_message(&flat),
        "flattened TTS Unauthorized must classify as session expiry (demoted, \
         not a hard error): {flat}"
    );
}

#[test]
fn tts_non_auth_error_is_not_demoted_to_session_expiry() {
    // A genuine TTS failure (timeout, 5xx, …) must keep its full anyhow chain
    // and NOT be demoted — real backend/TTS breakage must still reach Sentry.
    let flat = crate::client::flatten_authed_error(
        anyhow::anyhow!("connect timeout").context("backend request POST /openai/v1/audio/speech"),
    );

    assert!(
        !flat.contains("SESSION_EXPIRED"),
        "non-auth TTS error must not be demoted: {flat}"
    );
    assert!(
        flat.contains("connect timeout"),
        "underlying cause preserved: {flat}"
    );
    assert!(
        !openhuman_embed::__host::core::observability::is_session_expired_message(&flat),
        "non-auth TTS error must NOT classify as session expiry: {flat}"
    );
}
