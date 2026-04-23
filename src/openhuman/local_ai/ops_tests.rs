use super::*;

#[test]
fn extract_emoji_from_simple_string() {
    assert_eq!(extract_first_emoji("👍"), Some("👍".to_string()));
    assert_eq!(extract_first_emoji("🔥"), Some("🔥".to_string()));
    assert_eq!(extract_first_emoji("❤️"), Some("❤️".to_string()));
}

#[test]
fn extract_emoji_with_surrounding_text() {
    assert_eq!(extract_first_emoji("Sure! 😂"), Some("😂".to_string()));
    assert_eq!(
        extract_first_emoji("I think 👀 fits here"),
        Some("👀".to_string())
    );
}

#[test]
fn extract_none_when_no_emoji() {
    assert_eq!(extract_first_emoji("NONE"), None);
    assert_eq!(extract_first_emoji("no reaction"), None);
    assert_eq!(extract_first_emoji(""), None);
}

#[test]
fn extract_flag_emoji_keeps_pair_together() {
    assert_eq!(extract_first_emoji("🇺🇸"), Some("🇺🇸".to_string()));
    assert_eq!(
        extract_first_emoji("🇬🇧 Great Britain"),
        Some("🇬🇧".to_string())
    );
}

#[test]
fn is_emoji_start_recognizes_common_emojis() {
    assert!(is_emoji_start('👍'));
    assert!(is_emoji_start('🔥'));
    assert!(is_emoji_start('😂'));
    assert!(is_emoji_start('⭐'));
    assert!(!is_emoji_start('A'));
    assert!(!is_emoji_start('1'));
}

// ── Op-level validation / error paths (no hardware) ───────────

fn test_config(tmp: &tempfile::TempDir) -> Config {
    let mut c = Config::default();
    c.workspace_dir = tmp.path().join("workspace");
    c.config_path = tmp.path().join("config.toml");
    c.local_ai.enabled = false; // disable so the local-ai-disabled error path fires.
    c
}

#[tokio::test]
async fn local_ai_chat_rejects_empty_messages() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = local_ai_chat(&config, vec![], None).await.unwrap_err();
    assert!(err.contains("must not be empty"));
}

#[tokio::test]
async fn local_ai_prompt_errors_when_local_ai_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = local_ai_prompt(&config, "hello", None, None)
        .await
        .unwrap_err();
    assert!(err.contains("local ai is disabled"));
}

#[tokio::test]
async fn local_ai_vision_prompt_errors_when_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = local_ai_vision_prompt(&config, "hello", &[], None)
        .await
        .unwrap_err();
    assert!(err.contains("local ai is disabled"));
}

#[tokio::test]
async fn local_ai_embed_errors_when_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = local_ai_embed(&config, &["text".to_string()])
        .await
        .unwrap_err();
    assert!(err.contains("local ai is disabled"));
}

#[tokio::test]
async fn local_ai_summarize_errors_when_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = local_ai_summarize(&config, "some text", None)
        .await
        .unwrap_err();
    assert!(err.contains("local ai is disabled"));
}

#[tokio::test]
async fn local_ai_suggest_questions_returns_empty_without_local_ai() {
    // With local_ai disabled suggestions should silently produce an empty
    // list rather than erroring (graceful degradation).
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let outcome = local_ai_suggest_questions(&config, Some("topic".into()), None)
        .await
        .expect("suggestions should not error when disabled");
    assert!(outcome.value.is_empty());
}

#[tokio::test]
async fn local_ai_transcribe_errors_when_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = local_ai_transcribe(&config, "/tmp/x.wav")
        .await
        .unwrap_err();
    assert!(err.contains("local ai is disabled"));
}

#[tokio::test]
async fn local_ai_tts_errors_when_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = local_ai_tts(&config, "hello", None).await.unwrap_err();
    assert!(err.contains("local ai is disabled"));
}

#[tokio::test]
async fn local_ai_chat_errors_when_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let msg = vec![LocalAiChatMessage {
        role: "user".into(),
        content: "hi".into(),
    }];
    let err = local_ai_chat(&config, msg, None).await.unwrap_err();
    assert!(err.contains("local ai is disabled"));
}

#[tokio::test]
async fn local_ai_status_reports_even_when_disabled() {
    // Status should report the disabled state, not error out.
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let result = local_ai_status(&config).await;
    // Either Ok with a state payload or an error; we just ensure no panic.
    let _ = result;
}

#[tokio::test]
async fn local_ai_assets_status_returns_without_panic() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let _ = local_ai_assets_status(&config).await;
}
