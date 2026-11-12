use super::*;
use crate::config::test_env::EnvVarGuard;

#[tokio::test]
async fn hallucination_screen_timeout_fails_open() {
    let result = screen_with_timeout(
        std::future::pending::<Result<bool, &'static str>>(),
        std::time::Duration::from_millis(1),
    )
    .await;

    assert!(!result, "a timed-out screen must pass the transcript through");
}

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

    let _guard =
        EnvVarGuard::locked_set_async("PIPER_BIN", &piper_stub.display().to_string()).await;

    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");

    let result = voice_status(&config).await.unwrap();
    assert!(result.value.piper_binary.is_some());
}

/// The core ships no hosted STT provider: with the hosted-backend layer not
/// installed, the default `cloud` engine does not resolve, and status says so
/// instead of claiming a provider that is not there.
#[tokio::test]
async fn voice_status_reports_the_hosted_engine_unavailable_until_it_is_registered() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");

    let result = voice_status(&config).await.unwrap();
    assert!(
        !result.value.stt_available,
        "no hosted provider is registered in the bare core"
    );
    assert_eq!(result.value.stt_engine, "cloud");
    assert!(result.value.stt_error.is_some());
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
    config.voice_server.stt_engine = crate::config::schema::SttEngine::Elevenlabs;

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
    config.voice_server.stt_engine = crate::config::schema::SttEngine::Elevenlabs;
    config
        .voice_providers
        .push(crate::config::schema::voice_providers::VoiceProviderCreds {
            slug: "elevenlabs".into(),
            endpoint: "https://api.elevenlabs.io/v1".into(),
            capability: crate::config::schema::voice_providers::VoiceCapability::Both,
            ..Default::default()
        });

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

/// Status must name the provider synthesis actually uses. The top-level
/// `tts_provider` override wins over the legacy `local_ai.tts_provider`
/// (see `effective_tts_provider`), so status must not report the legacy
/// value while synthesis goes elsewhere.
#[tokio::test]
async fn voice_status_reports_the_effective_tts_provider() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    config.tts_provider = Some("piper".to_string());
    config.local_ai.tts_provider = "cloud".to_string();

    let result = voice_status(&config).await.unwrap();
    assert_eq!(result.value.tts_provider, "piper");
}

/// STT provider that always returns a stock hallucination phrase.
struct StockPhraseSttProvider;

#[async_trait::async_trait]
impl crate::voice::SttProvider for StockPhraseSttProvider {
    fn name(&self) -> &'static str {
        "stock-phrase-stt"
    }

    async fn transcribe(
        &self,
        _config: &Config,
        _audio_base64: &str,
        _mime_type: Option<&str>,
        _file_name: Option<&str>,
        _language: Option<&str>,
    ) -> Result<Outcome<crate::voice::SttResult>, String> {
        Ok(Outcome::new(
            crate::voice::SttResult {
                text: "Thanks for watching!".into(),
                provider: "stock-phrase-stt".into(),
            },
            Vec::new(),
        ))
    }
}

fn always_hallucinated<'a>(
    _config: &'a Config,
    _text: &'a str,
) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> {
    Box::pin(async { true })
}

/// A config whose STT routes to [`StockPhraseSttProvider`] under `slug`.
fn stock_phrase_config(slug: &str) -> Config {
    crate::voice::factory::registry::register_stt_provider(slug, |_, _| {
        Ok(Box::new(StockPhraseSttProvider) as Box<dyn crate::voice::SttProvider>)
    });
    let mut config = Config::default();
    config.stt_provider = Some(slug.to_string());
    config
}

/// The file entry point must apply the same hallucination gate as the bytes
/// entry point: a flagged transcript comes back empty, with the raw text kept.
#[tokio::test]
async fn transcribe_path_drops_hallucinated_transcripts() {
    let slug = "stock-phrase-stt-path-test";
    let config = stock_phrase_config(slug);
    let tmp = tempfile::tempdir().expect("tempdir");
    let audio = tmp.path().join("clip.wav");
    std::fs::write(&audio, b"audio").expect("write audio");

    let result = transcribe_path_with(
        &config,
        &audio.display().to_string(),
        None,
        true,
        always_hallucinated,
    )
    .await;
    crate::voice::factory::registry::unregister_stt_provider(slug);

    let speech = result.expect("transcription succeeds").value;
    assert_eq!(speech.raw_text, "Thanks for watching!");
    assert_eq!(speech.text, "", "a hallucinated transcript must be dropped");
}

#[tokio::test]
async fn transcribe_bytes_drops_hallucinated_transcripts() {
    let slug = "stock-phrase-stt-bytes-test";
    let config = stock_phrase_config(slug);

    let result = transcribe_bytes_with(
        &config,
        b"audio",
        Some("wav".to_string()),
        None,
        true,
        always_hallucinated,
    )
    .await;
    crate::voice::factory::registry::unregister_stt_provider(slug);

    let speech = result.expect("transcription succeeds").value;
    assert_eq!(speech.raw_text, "Thanks for watching!");
    assert_eq!(speech.text, "", "a hallucinated transcript must be dropped");
}
