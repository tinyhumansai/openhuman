use super::*;

#[test]
fn voice_speech_result_serializes_correctly() {
    let r = VoiceSpeechResult {
        text: "hello world".into(),
        raw_text: "hello world um".into(),
        model_id: "ggml-tiny-q5_1.bin".into(),
    };
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["text"], "hello world");
    assert_eq!(v["raw_text"], "hello world um");
    assert_eq!(v["model_id"], "ggml-tiny-q5_1.bin");
}

#[test]
fn voice_tts_result_serializes_correctly() {
    let r = VoiceTtsResult {
        output_path: "/tmp/out.wav".into(),
        voice_id: "en_US-lessac-medium".into(),
    };
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["output_path"], "/tmp/out.wav");
    assert_eq!(v["voice_id"], "en_US-lessac-medium");
}

#[test]
fn voice_status_serializes_correctly() {
    let s = VoiceStatus {
        stt_available: true,
        tts_available: false,
        stt_model_id: "tiny.bin".into(),
        tts_voice_id: "en_US-lessac-medium".into(),
        piper_binary: None,
        tts_voice_path: None,
        llm_cleanup_enabled: true,
        stt_engine: "elevenlabs".into(),
        stt_error: None,
        tts_provider: "cloud".into(),
    };
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["stt_available"], true);
    assert_eq!(v["tts_available"], false);
    assert!(v["piper_binary"].is_null());
    assert_eq!(v["llm_cleanup_enabled"], true);
    assert_eq!(v["stt_engine"], "elevenlabs");
    assert!(v["stt_error"].is_null());
    assert_eq!(v["tts_provider"], "cloud");
}

#[test]
fn from_local_ai_speech_result() {
    let local = LocalAiSpeechResult {
        text: "test".into(),
        model_id: "tiny".into(),
    };
    let voice: VoiceSpeechResult = local.into();
    assert_eq!(voice.text, "test");
    assert_eq!(voice.raw_text, "test");
    assert_eq!(voice.model_id, "tiny");
}

#[test]
fn from_local_ai_tts_result() {
    let local = LocalAiTtsResult {
        output_path: "/out.wav".into(),
        voice_id: "voice1".into(),
    };
    let voice: VoiceTtsResult = local.into();
    assert_eq!(voice.output_path, "/out.wav");
    assert_eq!(voice.voice_id, "voice1");
}

#[test]
fn serde_round_trip_speech_result() {
    let original = VoiceSpeechResult {
        text: "round trip".into(),
        raw_text: "round trip uh".into(),
        model_id: "model".into(),
    };
    let json = serde_json::to_string(&original).unwrap();
    let decoded: VoiceSpeechResult = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.text, original.text);
    assert_eq!(decoded.raw_text, original.raw_text);
    assert_eq!(decoded.model_id, original.model_id);
}
