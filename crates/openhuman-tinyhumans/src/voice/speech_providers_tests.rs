use super::*;
use openhuman_embed::__host::voice_registry::{
    create_stt_provider, create_tts_provider, unregister_stt_provider, unregister_tts_provider,
};

#[test]
fn registration_covers_every_historical_cloud_slug() {
    let _registry = crate::registry_test_lock();

    register();
    let config = Config::default();
    for slug in STT_SLUGS {
        let provider = create_stt_provider(slug, "ignored", &config).unwrap();
        assert_eq!(provider.name(), "cloud", "{slug}");
    }
    for slug in TTS_SLUGS {
        let provider = create_tts_provider(slug, "", &config).unwrap();
        assert_eq!(provider.name(), "cloud", "{slug}");
    }
    for slug in STT_SLUGS {
        unregister_stt_provider(slug);
    }
    for slug in TTS_SLUGS {
        unregister_tts_provider(slug);
    }
}

#[test]
fn cloud_tts_without_a_voice_carries_none_so_the_backend_default_applies() {
    // #5355: a Piper voice id must never reach the hosted proxy as `voice_id`.
    let provider = CloudTtsProvider::new(None);
    assert_eq!(provider.voice, None);
    let provider = CloudTtsProvider::new(Some("JBFqnCBsd6RMkjVDRZzb".into()));
    assert_eq!(provider.voice.as_deref(), Some("JBFqnCBsd6RMkjVDRZzb"));
}

/// The model the caller names reaches the provider; only an empty one falls
/// back to the proxy default.
#[test]
fn the_callers_stt_model_is_passed_through() {
    assert_eq!(stt_model("whisper-large"), "whisper-large");
    assert_eq!(stt_model("  whisper-large "), "whisper-large");
    assert_eq!(stt_model(""), DEFAULT_CLOUD_STT_MODEL);
    assert_eq!(stt_model("   "), DEFAULT_CLOUD_STT_MODEL);
}
