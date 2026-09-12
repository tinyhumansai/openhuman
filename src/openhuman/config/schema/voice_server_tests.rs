use super::*;

#[test]
fn defaults_are_opt_in_and_sane() {
    let c = VoiceServerConfig::default();
    // Always-on is privacy-sensitive — must default off.
    assert!(!c.always_on_enabled);
    // Onset must sit above the hotkey silence floor so an open mic rejects
    // ambient noise that the push-to-talk path would have tolerated.
    assert!(c.vad_onset_threshold > c.silence_threshold);
    assert!(c.vad_hangover_ms > 0);
    assert!(c.vad_min_speech_ms > 0);
    assert!(c.vad_max_utterance_secs > 0.0);
}

#[test]
fn stt_engine_defaults_to_backend_and_maps_to_routing_strings() {
    assert_eq!(VoiceServerConfig::default().stt_engine, SttEngine::Backend);
    assert_eq!(SttEngine::Backend.provider_string(), "cloud");
    assert_eq!(SttEngine::Elevenlabs.provider_string(), "elevenlabs");
    assert_eq!(SttEngine::Openai.provider_string(), "openai");
}

#[test]
fn stt_engine_parses_names_and_backend_aliases() {
    assert_eq!(SttEngine::parse("backend"), Some(SttEngine::Backend));
    // The routing grammar's pre-existing backend aliases must keep working
    // so an older `stt_provider = "cloud"` maps onto the same engine.
    assert_eq!(SttEngine::parse("cloud"), Some(SttEngine::Backend));
    assert_eq!(SttEngine::parse(" OpenHuman "), Some(SttEngine::Backend));
    assert_eq!(SttEngine::parse("ElevenLabs"), Some(SttEngine::Elevenlabs));
    assert_eq!(SttEngine::parse("openai"), Some(SttEngine::Openai));
    // The removed local engine is not an engine any more.
    assert_eq!(SttEngine::parse("whisper"), None);
    assert_eq!(SttEngine::parse("local"), None);
}

#[test]
fn deserializes_with_all_vad_fields_defaulted() {
    // An older config file with none of the Phase 2 keys must still load.
    let c: VoiceServerConfig = serde_json::from_str("{}").unwrap();
    assert!(!c.always_on_enabled);
    // A config file written before the engine selector existed must load.
    assert_eq!(c.stt_engine, SttEngine::Backend);
    assert_eq!(c.vad_hangover_ms, default_vad_hangover_ms());
    assert_eq!(c.vad_min_speech_ms, default_vad_min_speech_ms());
}
