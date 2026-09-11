use super::*;

#[test]
fn reserved_slugs() {
    for s in [
        "",
        " ",
        "cloud",
        "openhuman",
        "backend",
        "whisper",
        "local",
        "piper",
    ] {
        assert!(is_voice_slug_reserved(s), "{s:?} must be reserved");
    }
}

#[test]
fn non_reserved_slugs() {
    for s in ["deepgram", "elevenlabs", "openai", "groq", "my-custom"] {
        assert!(!is_voice_slug_reserved(s), "{s:?} must not be reserved");
    }
}

#[test]
fn generated_id_has_vp_prefix() {
    let id = generate_voice_provider_id("deepgram");
    assert!(id.starts_with("vp_deepgram_"), "got: {id}");
    assert_eq!(id.len(), "vp_deepgram_".len() + 5);
}

#[test]
fn generated_id_sanitises_slug() {
    let id = generate_voice_provider_id("my provider!");
    assert!(id.starts_with("vp_my_provider_"), "got: {id}");
}

#[test]
fn builtin_lookup_finds_known_slugs() {
    assert!(builtin_voice_provider("deepgram").is_some());
    assert!(builtin_voice_provider("elevenlabs").is_some());
    assert!(builtin_voice_provider("openai").is_some());
}

#[test]
fn builtin_lookup_misses_unknown() {
    assert!(builtin_voice_provider("groq").is_none());
}

#[test]
fn capability_helpers() {
    assert!(VoiceCapability::Stt.supports_stt());
    assert!(!VoiceCapability::Stt.supports_tts());
    assert!(!VoiceCapability::Tts.supports_stt());
    assert!(VoiceCapability::Tts.supports_tts());
    assert!(VoiceCapability::Both.supports_stt());
    assert!(VoiceCapability::Both.supports_tts());
}

#[test]
fn default_creds_round_trips() {
    let creds = VoiceProviderCreds::default();
    let json = serde_json::to_string(&creds).unwrap();
    let back: VoiceProviderCreds = serde_json::from_str(&json).unwrap();
    assert_eq!(creds, back);
}

#[test]
fn creds_with_fields_round_trips() {
    let creds = VoiceProviderCreds {
        id: "vp_deepgram_abc12".into(),
        slug: "deepgram".into(),
        label: "Deepgram".into(),
        endpoint: "https://api.deepgram.com/v1".into(),
        auth_style: AuthStyle::Bearer,
        capability: VoiceCapability::Stt,
        stt_api_style: SttApiStyle::Deepgram,
        tts_api_style: TtsApiStyle::OpenaiAudio,
        default_stt_model: Some("nova-2".into()),
        default_tts_voice: None,
    };
    let json = serde_json::to_string(&creds).unwrap();
    let back: VoiceProviderCreds = serde_json::from_str(&json).unwrap();
    assert_eq!(creds, back);
}
