use super::*;

/// TAURI-RUST-15BM: the hosted TTS route wrapping a refusal of the backend's
/// own ElevenLabs account.
const HOSTED_PAYMENT_TTS: &str = r#"POST /openai/v1/audio/speech failed (400 Bad Request): {"success":false,"error":"ElevenLabs request failed (401): {\"detail\":{\"status\":\"payment_required\",\"message\":\"Your subscription has a failed or incomplete payment\"}}","errorCode":"BAD_REQUEST"}"#;
/// TAURI-RUST-15CT: the same refusal on the hosted transcription route.
/// The backend's transcription wrapper differs from the TTS one.
const HOSTED_PAYMENT_STT: &str = r#"transcription request failed (400 Bad Request): {"success":false,"error":"ElevenLabs transcription failed (401): {\"detail\":{\"status\":\"payment_required\",\"message\":\"Your subscription has a failed or incomplete payment\"}}","errorCode":"BAD_REQUEST"}"#;
/// Streaming TTS uses a third wrapper.
const HOSTED_PAYMENT_STREAM_TTS: &str =
    r#"ElevenLabs streaming TTS failed (401): {"detail":{"status":"payment_required"}}"#;
/// A BYOK request refused because the user's own subscription is unpaid, in the
/// ElevenLabs client's format (`[voice-tts] elevenlabs error {status}: {body}`).
const BYOK_PAYMENT_TTS: &str = r#"[voice-tts] elevenlabs error 401 Unauthorized: {"detail":{"status":"payment_required","message":"Your subscription has a failed or incomplete payment"}}"#;
/// TAURI-RUST-1635: a BYOK user pasted the key ID.
const KEY_ID_STT: &str = r#"[voice-stt] elevenlabs error 400 Bad Request: {"detail":{"status":"invalid_api_key","message":"API key ID used as API key - only valid API keys can be used. API keys start with 'sk_'"}}"#;

#[test]
fn byok_payment_required_maps_and_demotes() {
    assert_eq!(
        voice_provider_user_message(BYOK_PAYMENT_TTS),
        Some(VOICE_PROVIDER_PAYMENT_REQUIRED_USER_MESSAGE)
    );
    assert_eq!(
        expected_error_kind(BYOK_PAYMENT_TTS),
        Some(ExpectedErrorKind::ProviderUserState)
    );
}

#[test]
fn key_id_maps_and_demotes() {
    assert_eq!(
        voice_provider_user_message(KEY_ID_STT),
        Some(VOICE_PROVIDER_KEY_ID_USER_MESSAGE)
    );
    assert_eq!(
        expected_error_kind(KEY_ID_STT),
        Some(ExpectedErrorKind::ProviderUserState)
    );
}

#[test]
fn hosted_payment_required_is_not_user_state() {
    // The backend's own account: no "update billing" message, and it keeps
    // reaching Sentry so ops sees it.
    for raw in [
        HOSTED_PAYMENT_TTS,
        HOSTED_PAYMENT_STT,
        HOSTED_PAYMENT_STREAM_TTS,
    ] {
        assert_eq!(voice_provider_user_message(raw), None, "{raw}");
        assert!(!is_voice_provider_user_state_message(raw), "{raw}");
        assert_ne!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ProviderUserState),
            "{raw}"
        );
        assert_eq!(map_voice_provider_error(raw.to_string()), raw);
    }
}

#[test]
fn mapped_user_messages_stay_demoted_at_rpc_boundary() {
    for m in [
        VOICE_PROVIDER_PAYMENT_REQUIRED_USER_MESSAGE,
        VOICE_PROVIDER_KEY_ID_USER_MESSAGE,
    ] {
        assert_eq!(
            expected_error_kind(m),
            Some(ExpectedErrorKind::ProviderUserState),
            "{m}"
        );
    }
}

#[test]
fn map_voice_provider_error_passes_unknown_through() {
    assert_eq!(
        map_voice_provider_error(BYOK_PAYMENT_TTS.to_string()),
        VOICE_PROVIDER_PAYMENT_REQUIRED_USER_MESSAGE
    );
    let other = "[voice-tts] elevenlabs 500: upstream exploded";
    assert_eq!(map_voice_provider_error(other.to_string()), other);
    assert_eq!(expected_error_kind(other), None);
}

#[test]
fn unrelated_payment_required_is_not_claimed() {
    assert_eq!(voice_provider_user_message("stripe payment_required"), None);
}

#[test]
fn other_byok_providers_are_not_claimed() {
    // OpenAI-compatible and Deepgram styles share the external clients but use
    // their own prefixes; the ElevenLabs guidance must not reach them.
    for raw in [
        r#"[voice-stt] external STT error 400 Bad Request: API key ID used as API key"#,
        r#"[voice-tts] external TTS error 402 Payment Required: payment_required"#,
        r#"[voice-stt] deepgram error 401 Unauthorized: API key ID used as API key"#,
    ] {
        assert_eq!(voice_provider_user_message(raw), None, "{raw}");
        assert_ne!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ProviderUserState),
            "{raw}"
        );
        assert_eq!(map_voice_provider_error(raw.to_string()), raw);
    }
}
