use super::*;

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
fn deserializes_with_voice_fields_defaulted() {
    let c: VoiceServerConfig = serde_json::from_str("{}").unwrap();
    // A config file written before the engine selector existed must load.
    assert_eq!(c.stt_engine, SttEngine::Backend);
}
