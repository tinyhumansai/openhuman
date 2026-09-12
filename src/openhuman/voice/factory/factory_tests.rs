//! Unit tests for the voice factory.

use super::entry::{
    create_stt_provider, create_tts_provider, default_stt_provider, default_tts_provider,
    resolve_tts_voice, DEFAULT_PIPER_VOICE,
};
use super::helpers::{effective_stt_provider, effective_tts_provider, split_slug_model};
use crate::openhuman::config::schema::voice_providers::{SttApiStyle, VoiceCapability};
use crate::openhuman::config::schema::SttEngine;
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
fn stt_factory_backend_alias() {
    let p = create_stt_provider("backend", "ignored", &cfg()).unwrap();
    assert_eq!(p.name(), "cloud");
}

/// The bundled whisper.cpp engine is gone, so `"whisper"` is no longer a
/// sentinel: it falls through to the slug lookup and errors by name. A silent
/// remap to cloud would hide an unmigrated config instead of surfacing it.
#[test]
fn stt_factory_removed_local_engine_errors_rather_than_falling_back() {
    for removed in ["whisper", "local"] {
        let err = create_stt_provider(removed, "", &cfg())
            .err()
            .unwrap_or_else(|| panic!("`{removed}` must not resolve to a provider"))
            .to_string();
        assert!(err.contains(removed), "error should name the string: {err}");
        assert!(
            err.contains("no voice provider"),
            "should fall through to the slug lookup: {err}"
        );
    }
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
    assert_eq!(p.configured_model(), Some("whisper-1"));
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

// ---------------------------------------------------------------------------
// Voice resolution (#5355)
//
// The regression: `create_tts_provider` coerced an empty voice to
// `DEFAULT_PIPER_VOICE` *before* the provider match, so every provider got a
// Piper model id. The cloud branch forwarded it as `voice_id` to the backend
// ElevenLabs proxy, which answers 400 Bad Request. The three RPC handlers that
// deliberately pass "" for non-Piper providers were silently overridden.
// ---------------------------------------------------------------------------

#[test]
fn tts_voice_empty_never_yields_piper_default_for_cloud() {
    // The #5355 regression, stated directly: no Piper voice id may escape to
    // the cloud provider. `None` makes `synthesize_reply` omit `voice_id` so
    // the backend applies its own default.
    for provider in ["cloud", "openhuman"] {
        assert_eq!(
            resolve_tts_voice(provider, ""),
            None,
            "{provider} with an empty voice must defer to the backend default"
        );
        assert_eq!(
            resolve_tts_voice(provider, "   "),
            None,
            "{provider} must treat a whitespace-only voice as empty"
        );
    }
}

#[test]
fn tts_voice_explicit_is_preserved_for_cloud() {
    // An explicit cloud voice (e.g. an ElevenLabs id) must still pass through
    // untouched — the fix must not swallow caller-supplied voices.
    assert_eq!(
        resolve_tts_voice("cloud", "JBFqnCBsd6RMkjVDRZzb"),
        Some("JBFqnCBsd6RMkjVDRZzb")
    );
    assert_eq!(resolve_tts_voice("cloud", "  Rachel  "), Some("Rachel"));
}

#[test]
fn tts_voice_piper_still_defaults_to_bundled_voice() {
    // Piper is the one provider the constant is valid for; this behaviour is
    // unchanged by the fix.
    assert_eq!(
        resolve_tts_voice("piper", ""),
        Some(DEFAULT_PIPER_VOICE),
        "piper must keep its bundled-voice fallback"
    );
    assert_eq!(
        resolve_tts_voice("piper", "en_GB-alba-medium"),
        Some("en_GB-alba-medium")
    );
}

#[test]
fn tts_voice_slug_empty_defers_to_registry_default() {
    // Second defect on the same line: the Piper id was never empty by the time
    // `create_tts_provider_by_slug` looked at it, so an external provider's
    // configured `default_tts_voice` could never win.
    assert_eq!(resolve_tts_voice("openai", ""), None);
    assert_eq!(resolve_tts_voice("elevenlabs", "   "), None);
}

#[test]
fn tts_voice_slug_suffix_beats_voice_argument() {
    // `slug:voice` is the more specific request, so it outranks the ambient
    // `voice` argument — same precedence the STT model resolution uses.
    assert_eq!(
        resolve_tts_voice("openai:shimmer", "alloy"),
        Some("shimmer")
    );
    assert_eq!(resolve_tts_voice("openai", "alloy"), Some("alloy"));
}

#[test]
fn tts_voice_resolution_tolerates_untrimmed_provider() {
    // A padded provider name must not fall through to the slug branch and be
    // treated as an unknown external provider.
    assert_eq!(resolve_tts_voice("  cloud  ", ""), None);
    assert_eq!(
        resolve_tts_voice("  piper  ", ""),
        Some(DEFAULT_PIPER_VOICE)
    );
}

#[test]
fn tts_factory_slug_empty_voice_uses_registry_default() {
    // End-to-end through the factory: with no voice anywhere, the registry
    // entry's `default_tts_voice` must be what the provider is built with —
    // previously pre-empted by the Piper id. Asserting the voice (not just
    // `name()`) is the point: a factory that passed a hardcoded value instead
    // of `resolved.unwrap_or("")` would still produce an "external" provider.
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
    let p = create_tts_provider("openai", "", &config).unwrap();
    assert_eq!(p.name(), "external");
    assert_eq!(
        p.configured_voice(),
        Some("alloy"),
        "registry default_tts_voice must reach the provider, not the Piper id"
    );
}

#[test]
fn tts_factory_cloud_empty_voice_carries_no_voice() {
    // The #5355 regression, asserted end-to-end through the boxed provider:
    // nothing but `None` may reach `CloudTtsProvider`, or `synthesize_reply`
    // puts a Piper id in `voice_id` and the backend answers 400.
    for provider in ["cloud", "openhuman"] {
        let p = create_tts_provider(provider, "", &cfg()).unwrap();
        assert_eq!(p.name(), "cloud");
        assert_eq!(
            p.configured_voice(),
            None,
            "{provider} must carry no voice so the backend default applies"
        );
    }
}

#[test]
fn tts_factory_cloud_preserves_explicit_voice() {
    let p = create_tts_provider("cloud", "JBFqnCBsd6RMkjVDRZzb", &cfg()).unwrap();
    assert_eq!(p.configured_voice(), Some("JBFqnCBsd6RMkjVDRZzb"));
}

#[test]
fn tts_factory_piper_empty_voice_carries_bundled_voice() {
    // The other half of the guarantee: Piper — and only Piper — still gets the
    // bundled voice id when the caller supplies none.
    let p = create_tts_provider("piper", "", &cfg()).unwrap();
    assert_eq!(p.name(), "piper");
    assert_eq!(p.configured_voice(), Some(DEFAULT_PIPER_VOICE));
}

#[test]
fn tts_factory_slug_colon_voice_reaches_provider() {
    // `slug:voice` must beat the registry default end-to-end.
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
    assert_eq!(p.configured_voice(), Some("shimmer"));
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
    config.local_ai.stt_provider = "openai".into();
    assert_eq!(effective_stt_provider(&config), "deepgram:nova-2");
}

#[test]
fn effective_stt_falls_back_to_legacy() {
    let mut config = cfg();
    config.stt_provider = None;
    config.local_ai.stt_provider = "deepgram:nova-2".into();
    assert_eq!(effective_stt_provider(&config), "deepgram:nova-2");
}

#[test]
fn effective_stt_defaults_to_cloud() {
    let mut config = cfg();
    config.stt_provider = None;
    config.local_ai.stt_provider = String::new();
    assert_eq!(effective_stt_provider(&config), "cloud");
}

/// `local_ai.stt_provider` has defaulted to the literal `"cloud"` since long
/// before engines existed, so treating it as an explicit choice would make the
/// engine picker a no-op for every existing workspace.
#[test]
fn effective_stt_engine_wins_over_default_cloud_routing_strings() {
    let mut config = cfg();
    config.voice_server.stt_engine = SttEngine::Openai;
    config.stt_provider = None;
    config.local_ai.stt_provider = "cloud".into();
    assert_eq!(effective_stt_provider(&config), "openai");

    config.stt_provider = Some("openhuman".into());
    assert_eq!(effective_stt_provider(&config), "openai");
}

/// A specific provider still outranks the engine field — otherwise a user who
/// pinned `deepgram:nova-2` would be silently moved onto the engine picker.
#[test]
fn effective_stt_explicit_slug_outranks_engine() {
    let mut config = cfg();
    config.voice_server.stt_engine = SttEngine::Elevenlabs;
    config.stt_provider = Some("deepgram:nova-2".into());
    assert_eq!(effective_stt_provider(&config), "deepgram:nova-2");
}

/// Removed local slugs are only rewritten by the config migration. A manually
/// reintroduced value must reach the factory and fail by name rather than being
/// silently routed to an engine the user did not select.
#[test]
fn effective_stt_unmigrated_local_value_is_not_a_sentinel() {
    let mut config = cfg();
    config.stt_provider = Some("whisper".into());
    config.local_ai.stt_provider = "whisper".into();
    config.voice_server.stt_engine = SttEngine::Elevenlabs;
    assert_eq!(effective_stt_provider(&config), "whisper");
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
