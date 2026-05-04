use super::*;
use crate::openhuman::voice::audio_capture::RecordingResult;

#[test]
fn default_server_config() {
    let cfg = VoiceServerConfig::default();
    assert_eq!(cfg.hotkey, "Fn");
    assert_eq!(cfg.activation_mode, ActivationMode::Push);
    assert!(!cfg.skip_cleanup);
    assert!(cfg.context.is_none());
    assert!(cfg.custom_dictionary.is_empty());
    assert!((cfg.silence_threshold - DEFAULT_SILENCE_THRESHOLD).abs() < 1e-6);
}

#[test]
fn hallucination_detection() {
    use super::HallucinationMode;
    let mode = HallucinationMode::Dictation;

    // Blank audio markers.
    assert!(is_hallucinated_output("[BLANK_AUDIO]", mode));
    assert!(is_hallucinated_output("  [blank_audio]  ", mode));
    assert!(is_hallucinated_output("[ BLANK_AUDIO ]", mode));
    // Common hallucinated phrases.
    assert!(is_hallucinated_output("Thank you for watching", mode));
    assert!(is_hallucinated_output("thanks for listening", mode));
    assert!(is_hallucinated_output("Thank you.", mode));
    assert!(is_hallucinated_output("Thank you", mode));
    assert!(is_hallucinated_output("Thanks.", mode));
    assert!(is_hallucinated_output("Bye.", mode));
    assert!(is_hallucinated_output("Goodbye.", mode));
    // Repeated words.
    assert!(is_hallucinated_output("you you you you", mode));
    assert!(is_hallucinated_output("the the the the", mode));
    // Punctuation-only.
    assert!(is_hallucinated_output("...", mode));
    assert!(is_hallucinated_output(".", mode));
    // Single noise words (dictation mode drops these).
    assert!(is_hallucinated_output("you", mode));
    assert!(is_hallucinated_output("Yeah", mode));
    assert!(is_hallucinated_output("Hmm", mode));
    assert!(is_hallucinated_output("Oh.", mode));
    // Should NOT flag real speech.
    assert!(!is_hallucinated_output("Hello, how are you?", mode));
    assert!(!is_hallucinated_output("the quick brown fox", mode));
    assert!(!is_hallucinated_output("I want to order pizza", mode));
    assert!(!is_hallucinated_output(
        "thank you for your help with the project",
        mode
    ));
    assert!(!is_hallucinated_output("", mode));
}

#[tokio::test]
async fn server_status_initial() {
    let server = VoiceServer::new(VoiceServerConfig::default());
    let status = server.status().await;
    assert_eq!(status.state, ServerState::Stopped);
    assert_eq!(status.transcription_count, 0);
    assert!(status.last_error.is_none());
}

#[tokio::test]
async fn stale_processing_cannot_reset_newer_recording_state() {
    let state = Arc::new(Mutex::new(ServerState::Recording));
    let session_generation = Arc::new(std::sync::atomic::AtomicU64::new(2));

    update_state_if_current(
        &state,
        &session_generation,
        1,
        ServerState::Idle,
        "stale_test",
    )
    .await;

    assert_eq!(*state.lock().await, ServerState::Recording);
}

#[tokio::test]
async fn current_processing_can_update_state() {
    let state = Arc::new(Mutex::new(ServerState::Recording));
    let session_generation = Arc::new(std::sync::atomic::AtomicU64::new(2));

    update_state_if_current(
        &state,
        &session_generation,
        2,
        ServerState::Idle,
        "current_test",
    )
    .await;

    assert_eq!(*state.lock().await, ServerState::Idle);
}

#[test]
fn server_state_serializes() {
    let json = serde_json::to_string(&ServerState::Recording).unwrap();
    assert_eq!(json, "\"recording\"");
}

#[test]
fn voice_server_status_serializes() {
    let status = VoiceServerStatus {
        state: ServerState::Idle,
        hotkey: "Fn".into(),
        activation_mode: ActivationMode::Push,
        transcription_count: 5,
        last_error: None,
    };
    let v = serde_json::to_value(&status).unwrap();
    assert_eq!(v["state"], "idle");
    assert_eq!(v["transcription_count"], 5);
}

#[test]
fn truncate_for_log_short() {
    assert_eq!(truncate_for_log("hello", 10), "hello");
}

#[test]
fn truncate_for_log_long() {
    let result = truncate_for_log("hello world this is long", 10);
    assert!(result.ends_with("..."));
    assert!(result.len() <= 14); // 10 + "..."
}

#[tokio::test]
async fn build_initial_prompt_combines_dictionary_and_recent_transcripts() {
    let config = VoiceServerConfig {
        custom_dictionary: vec!["OpenHuman".into(), "QuickJS".into()],
        ..VoiceServerConfig::default()
    };
    let recent = Mutex::new(vec!["first note".into(), "second note".into()]);

    let prompt = build_initial_prompt(&config, &recent)
        .await
        .expect("prompt should be built");

    assert!(prompt.contains("OpenHuman, QuickJS"));
    assert!(prompt.contains("first note second note"));
}

#[tokio::test]
async fn build_initial_prompt_truncates_on_char_boundary() {
    let repeated = "é".repeat(MAX_INITIAL_PROMPT_CHARS + 25);
    let config = VoiceServerConfig {
        custom_dictionary: vec![repeated],
        ..VoiceServerConfig::default()
    };
    let recent = Mutex::new(Vec::new());

    let prompt = build_initial_prompt(&config, &recent)
        .await
        .expect("prompt should be built");

    assert!(prompt.chars().count() <= MAX_INITIAL_PROMPT_CHARS);
    assert!(std::str::from_utf8(prompt.as_bytes()).is_ok());
}

#[tokio::test]
async fn push_recent_transcript_ignores_blank_and_caps_history() {
    let recent = Mutex::new(Vec::new());
    push_recent_transcript(&recent, "   ").await;
    assert!(recent.lock().await.is_empty());

    for idx in 0..(MAX_RECENT_TRANSCRIPTS + 2) {
        push_recent_transcript(&recent, &format!("line {idx}")).await;
    }

    let values = recent.lock().await.clone();
    assert_eq!(values.len(), MAX_RECENT_TRANSCRIPTS);
    assert_eq!(values.first().unwrap(), "line 2");
    assert_eq!(values.last().unwrap(), "line 6");
}

#[test]
fn capture_expected_app_name_is_none_off_macos() {
    if !cfg!(target_os = "macos") {
        assert_eq!(capture_expected_app_name(), None);
    }
}

#[tokio::test]
async fn process_recording_sets_last_error_when_stop_fails() {
    let handle = RecordingHandle::from_test_result(Err("stop failed".to_string()));

    let state = Arc::new(Mutex::new(ServerState::Recording));
    let last_error = Arc::new(Mutex::new(None));
    let generation = 1;
    let session_generation = Arc::new(std::sync::atomic::AtomicU64::new(generation));

    process_recording_bg(
        "test",
        handle,
        &Config::default(),
        &VoiceServerConfig::default(),
        state.clone(),
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
        session_generation,
        generation,
        last_error.clone(),
        Arc::new(Mutex::new(Vec::new())),
        None,
    )
    .await;

    assert_eq!(*state.lock().await, ServerState::Idle);
    assert_eq!(last_error.lock().await.as_deref(), Some("stop failed"));
}

#[tokio::test]
async fn process_recording_short_audio_returns_to_idle_without_error() {
    let handle = RecordingHandle::from_test_result(Ok(RecordingResult {
        wav_bytes: vec![1, 2, 3],
        duration_secs: 0.1,
        sample_count: 3,
        peak_rms: 0.5,
    }));

    let state = Arc::new(Mutex::new(ServerState::Recording));
    let last_error = Arc::new(Mutex::new(None));
    let generation = 1;
    let session_generation = Arc::new(std::sync::atomic::AtomicU64::new(generation));

    process_recording_bg(
        "test",
        handle,
        &Config::default(),
        &VoiceServerConfig::default(),
        state.clone(),
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
        session_generation,
        generation,
        last_error.clone(),
        Arc::new(Mutex::new(Vec::new())),
        None,
    )
    .await;

    assert_eq!(*state.lock().await, ServerState::Idle);
    assert!(last_error.lock().await.is_none());
}

#[tokio::test]
async fn process_recording_silence_skips_transcription() {
    let handle = RecordingHandle::from_test_result(Ok(RecordingResult {
        wav_bytes: vec![1, 2, 3],
        duration_secs: 1.0,
        sample_count: 3,
        peak_rms: 0.0,
    }));

    let state = Arc::new(Mutex::new(ServerState::Recording));
    let last_error = Arc::new(Mutex::new(None));
    let generation = 1;
    let session_generation = Arc::new(std::sync::atomic::AtomicU64::new(generation));

    process_recording_bg(
        "test",
        handle,
        &Config::default(),
        &VoiceServerConfig::default(),
        state.clone(),
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
        session_generation,
        generation,
        last_error.clone(),
        Arc::new(Mutex::new(Vec::new())),
        None,
    )
    .await;

    assert_eq!(*state.lock().await, ServerState::Idle);
    assert!(last_error.lock().await.is_none());
}

// ── truncate_for_log ───────────────────────────────────────────

#[test]
fn truncate_for_log_passes_through_short_strings() {
    assert_eq!(truncate_for_log("hi", 10), "hi");
    assert_eq!(truncate_for_log("", 10), "");
}

#[test]
fn truncate_for_log_appends_ellipsis_when_truncated() {
    assert_eq!(truncate_for_log("abcdefghij", 5), "abcde...");
}

#[test]
fn truncate_for_log_handles_multibyte_chars() {
    // Each "日" is multi-byte but one `char` — truncate by char count.
    let out = truncate_for_log("日本語テスト", 3);
    assert_eq!(out, "日本語...");
}

// ── try_global_server / global_server ─────────────────────────

#[tokio::test]
async fn try_global_server_returns_some_after_global_server_initialized() {
    // `global_server` is OnceCell-backed; first call initialises it.
    let _ = global_server(VoiceServerConfig::default());
    assert!(try_global_server().is_some());
}

// ── ServerState transitions ───────────────────────────────────
// Initial-status coverage lives in `server_status_initial` above.

#[test]
fn hallucination_detection_longer_real_phrase_is_not_flagged() {
    // Real multi-word speech should not be classified as hallucination.
    let mode = HallucinationMode::Dictation;
    assert!(!is_hallucinated_output(
        "please summarise the meeting",
        mode
    ));
    assert!(!is_hallucinated_output("open the browser", mode));
}

#[test]
fn hallucination_detection_trailing_exclamation_still_flags_known_pattern() {
    // Periods are stripped in normalisation; other punctuation behaviour
    // depends on the pattern list — we just lock in that exclamation
    // after "Thank you" does not accidentally un-flag it.
    let mode = HallucinationMode::Dictation;
    assert!(is_hallucinated_output("Thank you!", mode));
}
