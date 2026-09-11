use super::*;

fn test_config() -> Config {
    let mut config = Config::default();
    config.workspace_dir = std::env::temp_dir().join("openhuman-audio-toolkit-tests");
    config.local_ai.tts_provider = "cloud".to_string();
    config
}

#[test]
fn resolve_format_defaults_to_mp3_for_cloud() {
    assert_eq!(resolve_format("cloud", None).unwrap(), AudioFormat::Mp3);
}

#[test]
fn resolve_format_defaults_to_wav_for_piper() {
    assert_eq!(resolve_format("piper", None).unwrap(), AudioFormat::Wav);
}

#[test]
fn resolve_format_rejects_mp3_for_piper() {
    let err = resolve_format("piper", Some(AudioFormat::Mp3)).unwrap_err();
    assert!(err.contains("only supports wav"));
}

#[test]
fn slugify_title_collapses_noise() {
    assert_eq!(
        slugify_title(" Weekly update: Q2 / AI! "),
        "weekly-update-q2-ai"
    );
    assert_eq!(slugify_title("###"), "podcast");
}

#[test]
fn resolve_output_path_rejects_parent_dir() {
    let err = resolve_output_path(
        Path::new("/tmp/workspace"),
        Some("../escape.mp3"),
        None,
        AudioFormat::Mp3,
    )
    .unwrap_err();
    assert!(err.contains("parent-directory traversal"));
}

#[test]
fn build_email_message_includes_attachment_name() {
    let config = test_config();
    let message = build_email_message(
        &config,
        "listener@example.com",
        "Podcast",
        "Attached.",
        "briefing.mp3",
        "audio/mpeg".parse().unwrap(),
        vec![1, 2, 3],
    )
    .unwrap();
    let wire = String::from_utf8_lossy(&message.formatted()).to_string();
    assert!(wire.contains("Subject: Podcast"));
    assert!(wire.contains("filename=\"briefing.mp3\""));
    assert!(wire.contains("Content-Type: audio/mpeg"));
}

#[test]
fn resolve_email_capture_dir_uses_workspace_when_e2e_feature_enabled() {
    let config = test_config();
    let capture = resolve_email_capture_dir(&config);
    #[cfg(feature = "e2e-test-support")]
    assert!(capture.unwrap().ends_with(DEFAULT_CAPTURE_DIR));
    #[cfg(not(feature = "e2e-test-support"))]
    assert!(capture.is_none());
}

#[test]
fn effective_voice_defaults_for_piper_only() {
    assert_eq!(
        effective_voice("piper", None).as_deref(),
        Some(DEFAULT_PIPER_VOICE)
    );
    assert!(effective_voice("cloud", None).is_none());
}

#[test]
fn enforce_audio_format_requires_matching_mime() {
    assert!(enforce_audio_format(AudioFormat::Mp3, "audio/mpeg").is_ok());
    assert!(enforce_audio_format(AudioFormat::Mp3, "audio/wav").is_err());
}

#[test]
fn decode_audio_payload_rejects_bad_base64() {
    let err = decode_audio_payload("not-base64").unwrap_err();
    assert!(err.contains("invalid audio_base64"));
}
