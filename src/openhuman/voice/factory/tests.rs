//! Unit tests for the voice factory.

use super::entry::{
    create_stt_provider, create_tts_provider, default_stt_provider, default_tts_provider,
    DEFAULT_WHISPER_MODEL, WHISPER_MODEL_PRESETS,
};
use super::helpers::{effective_stt_provider, effective_tts_provider, split_slug_model};
use super::stt_providers::WhisperSttProvider;
use super::traits::SttProvider;
use crate::openhuman::config::schema::voice_providers::{
    SttApiStyle, TtsApiStyle, VoiceCapability,
};
use crate::openhuman::config::Config;

fn cfg() -> Config {
    Config::default()
}

#[test]
fn stt_factory_cloud_branch() {
    let p = create_stt_provider("cloud", "ignored", &cfg()).unwrap();
    assert_eq!(p.name(), "cloud");
}

#[test]
fn stt_factory_whisper_branch() {
    let p = create_stt_provider("whisper", "whisper-large-v3-turbo", &cfg()).unwrap();
    assert_eq!(p.name(), "whisper");
}

#[test]
fn stt_factory_whisper_empty_model_uses_default() {
    // Empty model → default whisper-large-v3-turbo; constructor must not
    // reject an empty string with an opaque error.
    let p = create_stt_provider("whisper", "", &cfg()).unwrap();
    assert_eq!(p.name(), "whisper");
}

#[test]
fn stt_factory_openhuman_sentinel() {
    let p = create_stt_provider("openhuman", "ignored", &cfg()).unwrap();
    assert_eq!(p.name(), "cloud");
}

#[test]
fn stt_factory_slug_without_registry_errors() {
    let err = create_stt_provider("deepgram", "nova-2", &cfg())
        .err()
        .expect("deepgram without registry entry must error");
    let msg = err.to_string();
    assert!(msg.contains("deepgram"), "should name the slug: {msg}");
    assert!(
        msg.contains("no voice provider"),
        "should explain missing: {msg}"
    );
}

#[test]
fn stt_factory_slug_colon_model_resolves_with_registry() {
    let mut config = cfg();
    config.voice_providers.push(
        crate::openhuman::config::schema::voice_providers::VoiceProviderCreds {
            slug: "deepgram".into(),
            endpoint: "https://api.deepgram.com/v1".into(),
            capability: VoiceCapability::Stt,
            stt_api_style: SttApiStyle::Deepgram,
            ..Default::default()
        },
    );
    let p = create_stt_provider("deepgram:nova-2", "", &config).unwrap();
    assert_eq!(p.name(), "external");
}

#[test]
fn stt_factory_bare_slug_resolves_with_registry() {
    let mut config = cfg();
    config.voice_providers.push(
        crate::openhuman::config::schema::voice_providers::VoiceProviderCreds {
            slug: "openai".into(),
            endpoint: "https://api.openai.com/v1".into(),
            capability: VoiceCapability::Both,
            default_stt_model: Some("whisper-1".into()),
            ..Default::default()
        },
    );
    let p = create_stt_provider("openai", "", &config).unwrap();
    assert_eq!(p.name(), "external");
}

#[test]
fn stt_factory_tts_only_provider_rejects() {
    let mut config = cfg();
    config.voice_providers.push(
        crate::openhuman::config::schema::voice_providers::VoiceProviderCreds {
            slug: "elevenlabs".into(),
            endpoint: "https://api.elevenlabs.io/v1".into(),
            capability: VoiceCapability::Tts,
            ..Default::default()
        },
    );
    let err = create_stt_provider("elevenlabs", "model", &config)
        .err()
        .expect("TTS-only provider must reject STT");
    assert!(err.to_string().contains("does not support STT"));
}

#[test]
fn stt_factory_empty_string_errors() {
    let err = create_stt_provider("", "model", &cfg())
        .err()
        .expect("empty provider must error");
    assert!(err.to_string().contains("no voice provider"));
}

#[test]
fn tts_factory_cloud_branch() {
    let p = create_tts_provider("cloud", "Rachel", &cfg()).unwrap();
    assert_eq!(p.name(), "cloud");
}

#[test]
fn tts_factory_piper_branch() {
    let p = create_tts_provider("piper", "en_US-lessac-medium", &cfg()).unwrap();
    assert_eq!(p.name(), "piper");
}

#[test]
fn tts_factory_piper_empty_voice_uses_default() {
    let p = create_tts_provider("piper", "", &cfg()).unwrap();
    assert_eq!(p.name(), "piper");
}

#[test]
fn tts_factory_openhuman_sentinel() {
    let p = create_tts_provider("openhuman", "alloy", &cfg()).unwrap();
    assert_eq!(p.name(), "cloud");
}

#[test]
fn tts_factory_slug_without_registry_errors() {
    let err = create_tts_provider("kokoro", "af_bella", &cfg())
        .err()
        .expect("kokoro without registry entry must error");
    let msg = err.to_string();
    assert!(msg.contains("kokoro"), "should name the slug: {msg}");
    assert!(
        msg.contains("no voice provider"),
        "should explain missing: {msg}"
    );
}

#[test]
fn tts_factory_slug_colon_voice_resolves_with_registry() {
    let mut config = cfg();
    config.voice_providers.push(
        crate::openhuman::config::schema::voice_providers::VoiceProviderCreds {
            slug: "openai".into(),
            endpoint: "https://api.openai.com/v1".into(),
            capability: VoiceCapability::Both,
            default_tts_voice: Some("alloy".into()),
            ..Default::default()
        },
    );
    let p = create_tts_provider("openai:shimmer", "", &config).unwrap();
    assert_eq!(p.name(), "external");
}

#[test]
fn tts_factory_stt_only_provider_rejects() {
    let mut config = cfg();
    config.voice_providers.push(
        crate::openhuman::config::schema::voice_providers::VoiceProviderCreds {
            slug: "deepgram".into(),
            endpoint: "https://api.deepgram.com/v1".into(),
            capability: VoiceCapability::Stt,
            ..Default::default()
        },
    );
    let err = create_tts_provider("deepgram", "voice", &config)
        .err()
        .expect("STT-only provider must reject TTS");
    assert!(err.to_string().contains("does not support TTS"));
}

#[test]
fn whisper_presets_cover_full_size_ladder() {
    // Sanity-check the installer surface: tiny→large-v3-turbo must all be
    // exposed so the local-AI panel can render the size picker without
    // hard-coding the list.
    let ids: Vec<&str> = WHISPER_MODEL_PRESETS.iter().map(|(id, _)| *id).collect();
    for expected in ["tiny", "base", "small", "medium", "large-v3-turbo"] {
        assert!(
            ids.contains(&expected),
            "WHISPER_MODEL_PRESETS missing {expected}"
        );
    }
}

#[tokio::test]
async fn whisper_provider_fails_clearly_when_binary_missing() {
    // No WHISPER_BIN env, no model file — the provider must surface an
    // actionable error rather than panic. Drive a small base64 payload
    // so we never reach the actual transcription call.
    let _guard = unset_env_guard("WHISPER_BIN");
    let provider = WhisperSttProvider::new("whisper-large-v3-turbo");
    let result = provider
        .transcribe(&cfg(), "AAAA", Some("audio/wav"), None, None)
        .await;
    assert!(result.is_err(), "missing binary must error");
    let msg = result.err().unwrap();
    // Whatever the underlying message says, it must NOT be a serialize
    // panic — i.e. we must have hit the binary-resolution branch.
    assert!(
        !msg.is_empty(),
        "error message should be populated for diagnosis"
    );
}

#[test]
fn default_providers_return_cloud() {
    assert_eq!(default_stt_provider().name(), "cloud");
    assert_eq!(default_tts_provider().name(), "cloud");
}

// ── slug:model parsing ──────────────────────────────────────────────

#[test]
fn split_slug_model_with_colon() {
    assert_eq!(split_slug_model("deepgram:nova-2"), ("deepgram", "nova-2"));
}

#[test]
fn split_slug_model_bare_slug() {
    assert_eq!(split_slug_model("deepgram"), ("deepgram", ""));
}

#[test]
fn split_slug_model_multiple_colons() {
    assert_eq!(split_slug_model("custom:model:v2"), ("custom", "model:v2"));
}

// ── effective provider resolution ───────────────────────────────────

#[test]
fn effective_stt_prefers_new_field() {
    let mut config = cfg();
    config.stt_provider = Some("deepgram:nova-2".into());
    config.local_ai.stt_provider = "whisper".into();
    assert_eq!(effective_stt_provider(&config), "deepgram:nova-2");
}

#[test]
fn effective_stt_falls_back_to_legacy() {
    let mut config = cfg();
    config.stt_provider = None;
    config.local_ai.stt_provider = "whisper".into();
    assert_eq!(effective_stt_provider(&config), "whisper");
}

#[test]
fn effective_stt_defaults_to_cloud() {
    let mut config = cfg();
    config.stt_provider = None;
    config.local_ai.stt_provider = String::new();
    assert_eq!(effective_stt_provider(&config), "cloud");
}

#[test]
fn effective_tts_prefers_new_field() {
    let mut config = cfg();
    config.tts_provider = Some("openai:alloy".into());
    config.local_ai.tts_provider = "piper".into();
    assert_eq!(effective_tts_provider(&config), "openai:alloy");
}

#[test]
fn effective_tts_falls_back_to_legacy() {
    let mut config = cfg();
    config.tts_provider = None;
    config.local_ai.tts_provider = "piper".into();
    assert_eq!(effective_tts_provider(&config), "piper");
}

#[test]
fn effective_tts_defaults_to_cloud() {
    let config = cfg();
    assert_eq!(effective_tts_provider(&config), "cloud");
}

/// Drop guard that unsets an env var on construction and restores it on
/// drop. Necessary because cargo runs tests in parallel and bare
/// `remove_var` would leak across tests.
fn unset_env_guard(key: &'static str) -> EnvUnsetGuard {
    let prev = std::env::var_os(key);
    std::env::remove_var(key);
    EnvUnsetGuard { key, prev }
}

struct EnvUnsetGuard {
    key: &'static str,
    prev: Option<std::ffi::OsString>,
}
impl Drop for EnvUnsetGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}
