use super::*;
use crate::config::schema::voice_live::{
    LIVE_PROVIDERS, LIVE_PROVIDER_ELEVENLABS_HOSTED, LIVE_PROVIDER_GEMINI_HOSTED,
};

#[test]
fn key_slugs_cover_only_byok_providers() {
    assert_eq!(key_slug(LIVE_PROVIDER_GEMINI), Some("google"));
    assert_eq!(key_slug(LIVE_PROVIDER_SARVAM), Some("sarvam"));
    assert_eq!(key_slug(LIVE_PROVIDER_GEMINI_HOSTED), None);
    assert_eq!(key_slug(LIVE_PROVIDER_ELEVENLABS_HOSTED), None);
}

#[test]
fn catalogue_lists_only_the_byok_providers_when_no_hosted_one_is_registered() {
    let config = Config::default();
    let infos = provider_infos(&config);
    let ids: Vec<_> = infos.iter().map(|i| i.id.as_str()).collect();
    let byok: Vec<_> = LIVE_PROVIDERS
        .iter()
        .copied()
        .filter(|id| key_slug(id).is_some())
        .collect();
    assert_eq!(ids, byok);
    assert!(!ids.contains(&LIVE_PROVIDER_GEMINI_HOSTED));
    let sarvam = infos.iter().find(|i| i.id == LIVE_PROVIDER_SARVAM).unwrap();
    assert_eq!(sarvam.kind, LiveProviderKind::Byok);
    assert_eq!(sarvam.key_slug.as_deref(), Some("sarvam"));
    assert!(sarvam.languages.contains(&"hi-IN".to_string()));
    assert!(sarvam.voices.contains(&"shubh".to_string()));
}

#[test]
fn live_config_follows_the_settings_per_provider() {
    let mut config = Config::default();
    config.voice_live.gemini.voice = Some("Kore".into());
    config.voice_live.gemini.model = Some("gemini-x".into());
    config.voice_live.sarvam.speaker = Some("priya".into());

    let gemini = live_config(&config, LIVE_PROVIDER_GEMINI, "prompt", 16_000);
    assert_eq!(gemini.system_instruction.as_deref(), Some("prompt"));
    assert_eq!(gemini.voice.as_deref(), Some("Kore"));
    assert_eq!(gemini.model.as_deref(), Some("gemini-x"));
    assert!(gemini
        .provider_options
        .get("context_window_compression")
        .is_some());

    let sarvam = live_config(&config, LIVE_PROVIDER_SARVAM, "prompt", 8_000);
    assert_eq!(sarvam.voice.as_deref(), Some("priya"));
    assert_eq!(sarvam.language.as_deref(), Some("en-IN"));
    assert_eq!(sarvam.input_format.sample_rate, 8_000);
    config.voice_live.sarvam.language = Some("auto".into());
    let auto = live_config(&config, LIVE_PROVIDER_SARVAM, "prompt", 16_000);
    assert_eq!(auto.language, None);
    assert_eq!(auto.provider_options["auto_language"], true);

    let unknown = live_config(&config, "nope", "prompt", 16_000);
    assert_eq!(unknown.system_instruction, None);
}

#[tokio::test]
async fn byok_providers_need_a_stored_key() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.workspace_dir = dir.path().to_path_buf();
    config.config_path = dir.path().join("config.toml");
    for id in [LIVE_PROVIDER_GEMINI, LIVE_PROVIDER_SARVAM] {
        let err = prepare(&config, id, LiveConfig::new()).await.err().unwrap();
        assert_eq!(err.code, "not_configured", "{id}");
    }
    let err = prepare(&config, "nope", LiveConfig::new())
        .await
        .err()
        .unwrap();
    assert_eq!(err.code, "invalid_request");
}

#[tokio::test]
async fn hosted_providers_are_unknown_until_the_hosted_layer_registers_them() {
    let config = Config::default();
    for id in [LIVE_PROVIDER_GEMINI_HOSTED, LIVE_PROVIDER_ELEVENLABS_HOSTED] {
        let err = prepare(&config, id, LiveConfig::new()).await.err().unwrap();
        assert_eq!(err.code, "invalid_request", "{id}");
    }
}
