use super::*;

const PAYMENT_TTS: &str = r#"POST /openai/v1/audio/speech failed (400 Bad Request): ElevenLabs request failed (401): payment_required — "Your subscription has a failed or incomplete payment""#;
const KEY_ID_STT: &str = r#"[voice-stt] elevenlabs 400 invalid_api_key: "API key ID used as API key - only valid API keys can be used. API keys start with 'sk_'""#;

#[test]
fn payment_required_maps_and_demotes() {
    assert_eq!(
        voice_provider_user_message(PAYMENT_TTS),
        Some(VOICE_PROVIDER_PAYMENT_REQUIRED_USER_MESSAGE)
    );
    assert_eq!(
        expected_error_kind(PAYMENT_TTS),
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
        map_voice_provider_error(PAYMENT_TTS.to_string()),
        VOICE_PROVIDER_PAYMENT_REQUIRED_USER_MESSAGE
    );
    let other = "POST /openai/v1/audio/speech failed (400 Bad Request): {\"error\":\"x\"}";
    assert_eq!(map_voice_provider_error(other.to_string()), other);
    assert_eq!(expected_error_kind(other), None);
}

#[test]
fn unrelated_payment_required_is_not_claimed() {
    assert_eq!(voice_provider_user_message("stripe payment_required"), None);
}
