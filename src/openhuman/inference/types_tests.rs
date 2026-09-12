use super::*;

#[test]
fn disabled_status_marks_all_capabilities_disabled() {
    let config = Config::default();
    let status = LocalAiStatus::disabled(&config);

    assert_eq!(status.state, "disabled");
    assert_eq!(status.vision_state, "disabled");
    assert_eq!(status.embedding_state, "disabled");
    assert_eq!(status.stt_state, "disabled");
    assert_eq!(status.tts_state, "disabled");
    assert_eq!(status.provider, "ollama");
    assert_eq!(status.active_backend, "ollama");
}

#[test]
fn disabled_status_reflects_lm_studio_provider() {
    use crate::openhuman::inference::local::provider::LocalAiProvider;

    let mut config = Config::default();
    config.local_ai.provider = LocalAiProvider::LmStudio.as_str().to_string();
    let status = LocalAiStatus::disabled(&config);

    assert_eq!(status.provider, "lm_studio");
    assert_eq!(status.active_backend, "lm_studio");
}

#[test]
fn disabled_status_uses_config_vision_mode() {
    let mut config = Config::default();
    config.local_ai.chat_model_id = "gemma3:1b-it-qat".to_string();
    config.local_ai.vision_model_id.clear();
    config.local_ai.embedding_model_id = "all-minilm:latest".to_string();

    let status = LocalAiStatus::disabled(&config);
    assert_eq!(status.vision_mode, "disabled");
}
