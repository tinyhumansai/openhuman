use super::*;

#[test]
fn normalize_extension_defaults_to_webm() {
    assert_eq!(normalize_extension(None).unwrap(), "webm");
}

#[test]
fn normalize_extension_strips_dot_and_lowercases() {
    assert_eq!(
        normalize_extension(Some(".WebM".to_string())).unwrap(),
        "webm"
    );
    assert_eq!(normalize_extension(Some("OGG".to_string())).unwrap(), "ogg");
    assert_eq!(
        normalize_extension(Some("  .WAV  ".to_string())).unwrap(),
        "wav"
    );
}

#[test]
fn normalize_extension_accepts_alphanumeric() {
    assert_eq!(normalize_extension(Some("m4a".to_string())).unwrap(), "m4a");
    assert_eq!(normalize_extension(Some("mp3".to_string())).unwrap(), "mp3");
}

#[test]
fn normalize_extension_rejects_empty() {
    assert!(normalize_extension(Some("".to_string())).is_err());
    assert!(normalize_extension(Some("  ".to_string())).is_err());
    assert!(normalize_extension(Some(".".to_string())).is_err());
}

#[test]
fn normalize_extension_rejects_invalid_chars() {
    assert!(normalize_extension(Some("a/b".to_string())).is_err());
    assert!(normalize_extension(Some("web m".to_string())).is_err());
    assert!(normalize_extension(Some("a.b".to_string())).is_err());
}

#[tokio::test]
async fn voice_status_returns_without_error() {
    let config = Config::default();
    let result = voice_status(&config).await;
    assert!(result.is_ok());
    let status = result.unwrap().value;
    assert!(!status.stt_model_id.is_empty());
    assert!(!status.tts_voice_id.is_empty());
}

/// RAII guard that restores an env var on drop, even on panic.
struct EnvGuard {
    key: &'static str,
    prev: Option<std::ffi::OsString>,
}
impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, prev }
    }
}
impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

#[tokio::test]
async fn voice_status_detects_stub_binaries() {
    let tmp = tempfile::tempdir().expect("tempdir");

    let piper_stub = tmp.path().join("piper");
    std::fs::write(&piper_stub, b"#!/bin/sh\n").expect("write stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&piper_stub, std::fs::Permissions::from_mode(0o755))
            .expect("chmod");
    }

    let _guard = EnvGuard::set("PIPER_BIN", &piper_stub.display().to_string());

    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");

    let result = voice_status(&config).await.unwrap();
    assert!(result.value.piper_binary.is_some());
}

/// STT is hosted, so status must report it available on a default config
/// with no local binaries or models anywhere — the state that used to mean
/// "STT unavailable" back when a whisper.cpp install was required.
#[tokio::test]
async fn voice_status_reports_backend_stt_available_without_any_local_install() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");

    let result = voice_status(&config).await.unwrap();
    assert!(
        result.value.stt_available,
        "backend engine is always routable"
    );
    assert_eq!(result.value.stt_engine, "cloud");
    assert!(result.value.stt_error.is_none());
}

/// Selecting a third-party engine with no matching `voice_providers` entry
/// must surface as unavailable-with-a-reason rather than silently falling
/// back to the backend proxy and billing the wrong account.
#[tokio::test]
async fn voice_status_reports_unconfigured_engine_as_unavailable() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    config.voice_server.stt_engine = crate::openhuman::config::schema::SttEngine::Elevenlabs;

    let result = voice_status(&config).await.unwrap();
    assert!(!result.value.stt_available);
    assert_eq!(result.value.stt_engine, "elevenlabs");
    let err = result.value.stt_error.expect("reason must be reported");
    assert!(err.contains("elevenlabs"), "reason names the slug: {err}");
}

#[tokio::test]
async fn voice_status_reports_external_engine_without_credentials_as_unavailable() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    config.voice_server.stt_engine = crate::openhuman::config::schema::SttEngine::Elevenlabs;
    config.voice_providers.push(
        crate::openhuman::config::schema::voice_providers::VoiceProviderCreds {
            slug: "elevenlabs".into(),
            endpoint: "https://api.elevenlabs.io/v1".into(),
            capability: crate::openhuman::config::schema::voice_providers::VoiceCapability::Both,
            ..Default::default()
        },
    );

    let status = voice_status(&config).await.unwrap().value;
    assert!(!status.stt_available);
    assert!(status
        .stt_error
        .as_deref()
        .is_some_and(|error| error.contains("no API credential")));
}

#[test]
fn safe_basename_helpers_cover_missing_and_present_values() {
    assert_eq!(safe_basename_path(&None), "<none>");
    assert_eq!(safe_basename_str(&None), "<none>");

    let path = Some(std::path::PathBuf::from("/tmp/models/voice.bin"));
    let string = Some("/tmp/models/voice.bin".to_string());
    assert_eq!(safe_basename_path(&path), "voice.bin");
    assert_eq!(safe_basename_str(&string), "voice.bin");
}

#[tokio::test]
/// Transcription no longer depends on the local-AI runtime — STT is a
/// hosted call, so a workspace with `local_ai.runtime_enabled = false` must
/// still reach the engine. The only failure left on this path is the audio
/// file itself, and it must name the file rather than blaming local AI.
async fn voice_transcribe_errors_on_unreadable_audio_not_on_disabled_local_ai() {
    let mut config = Config::default();
    config.local_ai.runtime_enabled = false;
    let missing = std::env::temp_dir().join("openhuman-no-such-input.wav");
    let _ = std::fs::remove_file(&missing);

    let err = voice_transcribe(&config, &format!(" {} ", missing.display()), None, true)
        .await
        .expect_err("a missing audio file must fail");
    assert!(
        err.contains("failed to read audio file"),
        "error should name the unreadable file, got: {err}"
    );
    assert!(
        !err.contains("local ai is disabled"),
        "hosted STT must not be gated on the local-AI runtime: {err}"
    );
}

#[tokio::test]
/// Same contract for the bytes entry point: with local AI off it still
/// reaches the hosted engine, so the failure comes from the upload (no
/// signed-in session in a test workspace), never from a local-AI gate.
async fn voice_transcribe_bytes_is_not_gated_on_the_local_ai_runtime() {
    let mut config = Config::default();
    config.local_ai.runtime_enabled = false;

    let err = voice_transcribe_bytes(&config, b"abc", Some("wav".to_string()), None, true)
        .await
        .expect_err("no signed-in session means the upload cannot happen");
    assert!(
        !err.contains("local ai is disabled"),
        "hosted STT must not be gated on the local-AI runtime: {err}"
    );
}

#[tokio::test]
async fn voice_tts_errors_when_local_ai_disabled() {
    let mut config = Config::default();
    config.local_ai.runtime_enabled = false;

    let err = voice_tts(&config, "hello world", None)
        .await
        .expect_err("disabled local ai should fail");
    assert!(err.contains("local ai is disabled"));
}
