use super::*;

#[test]
fn defaults_to_the_hosted_gemini_provider() {
    let config = LiveVoiceConfig::default();
    assert_eq!(config.default_provider, LIVE_PROVIDER_GEMINI_HOSTED);
    assert_eq!(config.gemini, GeminiLiveSettings::default());
}

#[test]
fn deserializes_partial_tables_with_defaults() {
    let config: LiveVoiceConfig = toml::from_str("[sarvam]\nlanguage = \"hi-IN\"\n").unwrap();
    assert_eq!(config.default_provider, LIVE_PROVIDER_GEMINI_HOSTED);
    assert_eq!(config.sarvam.language.as_deref(), Some("hi-IN"));
    assert_eq!(config.sarvam.speaker, None);
}

#[test]
fn recognises_provider_ids() {
    for id in LIVE_PROVIDERS {
        assert!(is_live_provider(id));
    }
    assert!(!is_live_provider("openai"));
}
