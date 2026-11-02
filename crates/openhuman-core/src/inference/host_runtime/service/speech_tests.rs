use super::*;
#[cfg(feature = "voice")]
use crate::core::Outcome;
#[cfg(feature = "voice")]
use crate::voice::{SttProvider, SttResult};
#[cfg(feature = "voice")]
use async_trait::async_trait;
#[cfg(feature = "voice")]
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
#[cfg(feature = "voice")]
use std::sync::{Arc, Mutex};

#[test]
fn file_transcription_errors_keep_the_host_path_context() {
    let missing = restore_host_file_error(
        "[voice-stt] failed to read audio file: No such file or directory (os error 2)".into(),
        "/tmp/missing.wav",
    );
    assert_eq!(
        missing,
        "failed to read audio file /tmp/missing.wav: No such file or directory (os error 2)"
    );

    let empty = restore_host_file_error("[voice-stt] audio file is empty".into(), "/tmp/empty.wav");
    assert_eq!(empty, "audio file /tmp/empty.wav is empty");

    assert_eq!(
        restore_host_file_error("provider refused audio".into(), "/tmp/audio.wav"),
        "provider refused audio"
    );
}

#[derive(Debug, PartialEq, Eq)]
#[cfg(feature = "voice")]
struct CapturedAudio {
    bytes: Vec<u8>,
    mime_type: Option<String>,
    file_name: Option<String>,
}

#[cfg(feature = "voice")]
struct CapturingSttProvider(Arc<Mutex<Vec<CapturedAudio>>>);

#[cfg(feature = "voice")]
#[async_trait]
impl SttProvider for CapturingSttProvider {
    fn name(&self) -> &'static str {
        "capturing-stt"
    }

    async fn transcribe(
        &self,
        _config: &Config,
        audio_base64: &str,
        mime_type: Option<&str>,
        file_name: Option<&str>,
        _language: Option<&str>,
    ) -> Result<Outcome<SttResult>, String> {
        self.0.lock().unwrap().push(CapturedAudio {
            bytes: BASE64.decode(audio_base64).unwrap(),
            mime_type: mime_type.map(str::to_owned),
            file_name: file_name.map(str::to_owned),
        });
        Ok(Outcome::new(
            SttResult {
                text: "recognized speech".into(),
                provider: "capturing-stt".into(),
            },
            Vec::new(),
        ))
    }
}

#[cfg(feature = "voice")]
#[tokio::test]
async fn native_transcription_helpers_preserve_audio_metadata_for_host_stt() {
    let tmp = tempfile::tempdir().unwrap();
    let config = Config::default();
    let file_path = tmp.path().join("voice.wav");
    std::fs::write(&file_path, b"file-audio").unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider_calls = Arc::clone(&calls);
    crate::voice::factory::registry::register_stt_provider(
        "capturing-stt-host-test",
        move |_, _| {
            Ok(Box::new(CapturingSttProvider(Arc::clone(&provider_calls))) as Box<dyn SttProvider>)
        },
    );
    let provider = ConfiguredSttProvider {
        config: &config,
        provider_name: "capturing-stt-host-test".into(),
    };

    let file_text = tinyinference_voice::transcription::transcribe_file(&file_path, &provider)
        .await
        .unwrap();
    let bytes_text = tinyinference_voice::transcription::transcribe_bytes(
        b"buffer-audio",
        Some(" .WEBM "),
        &provider,
    )
    .await
    .unwrap();
    crate::voice::factory::registry::unregister_stt_provider("capturing-stt-host-test");

    assert_eq!(file_text, "recognized speech");
    assert_eq!(bytes_text, "recognized speech");
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            CapturedAudio {
                bytes: b"file-audio".to_vec(),
                mime_type: Some("audio/wav".into()),
                file_name: Some("voice.wav".into()),
            },
            CapturedAudio {
                bytes: b"buffer-audio".to_vec(),
                mime_type: Some("audio/webm".into()),
                file_name: Some("audio.webm".into()),
            },
        ]
    );
}

fn enabled_config(tmp: &tempfile::TempDir) -> Config {
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    config.local_ai.runtime_enabled = true;
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

#[tokio::test]
async fn tts_rejects_empty_text_before_resolving_piper() {
    let tmp = tempfile::tempdir().unwrap();
    let config = enabled_config(&tmp);
    let service = LocalAiService::new(&crate::inference::local_runtime_config(&config));

    let error = tts(&service, &config, " \n\t ", None).await.unwrap_err();

    assert!(
        error.contains("text is required"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn tts_rejects_output_outside_allowed_roots() {
    let tmp = tempfile::tempdir().unwrap();
    let config = enabled_config(&tmp);
    let service = LocalAiService::new(&crate::inference::local_runtime_config(&config));
    let output = tmp.path().join("outside").join("speech.wav");

    let error = tts(&service, &config, "hello", Some(output.to_str().unwrap()))
        .await
        .unwrap_err();

    assert!(error.contains("output_path"), "unexpected error: {error}");
    assert!(!output.parent().unwrap().exists());
}

#[tokio::test]
async fn tts_output_path_allows_workspace_and_local_ai_models_roots() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = enabled_config(&tmp);
    config.autonomy.enabled = true;
    config.autonomy.workspace_only = true;
    config.action_dir = tmp.path().join("action");
    std::fs::create_dir_all(&config.action_dir).unwrap();
    let runtime = crate::inference::local_runtime_config(&config);
    // `enabled_config` anchors `config_path` inside this fixture, so the host
    // resolves its local-AI models under the fixture root.
    let model_root = tmp.path().join("models").join("local-ai");

    for output in [
        config.workspace_dir.join("speech.wav"),
        model_root.join("speech.wav"),
    ] {
        let resolved = validated_tts_output_path(&config, &runtime, Some(output.to_str().unwrap()))
            .await
            .unwrap();
        assert!(resolved.ends_with("speech.wav"));
    }
}

#[tokio::test]
async fn tts_output_path_rejects_traversal() {
    let tmp = tempfile::tempdir().unwrap();
    let config = enabled_config(&tmp);
    let runtime = crate::inference::local_runtime_config(&config);
    let traversal = config.workspace_dir.join("../outside/speech.wav");

    let error = validated_tts_output_path(&config, &runtime, Some(traversal.to_str().unwrap()))
        .await
        .unwrap_err();

    assert!(error.contains("output_path"));
}

#[cfg(unix)]
#[tokio::test]
async fn tts_output_path_rejects_symlink_escape() {
    let tmp = tempfile::tempdir().unwrap();
    let config = enabled_config(&tmp);
    let runtime = crate::inference::local_runtime_config(&config);
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let link = config.workspace_dir.join("linked");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    let output = link.join("speech.wav");

    let error = validated_tts_output_path(&config, &runtime, Some(output.to_str().unwrap()))
        .await
        .unwrap_err();

    assert!(error.contains("output_path"));
}

#[cfg(unix)]
#[tokio::test]
async fn tts_output_path_rejects_final_component_symlink() {
    let tmp = tempfile::tempdir().unwrap();
    let config = enabled_config(&tmp);
    let runtime = crate::inference::local_runtime_config(&config);
    let outside_file = tmp.path().join("outside.wav");
    std::fs::write(&outside_file, b"outside target").unwrap();
    let output = config.workspace_dir.join("speech.wav");
    std::os::unix::fs::symlink(&outside_file, &output).unwrap();

    let error = validated_tts_output_path(&config, &runtime, Some(output.to_str().unwrap()))
        .await
        .unwrap_err();

    assert!(
        error.contains("symbolic links"),
        "unexpected error: {error}"
    );
    assert_eq!(std::fs::read(outside_file).unwrap(), b"outside target");
}

#[cfg(unix)]
#[tokio::test]
async fn tts_output_path_keeps_always_forbidden_paths_blocked() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.workspace_dir = std::path::PathBuf::from("/etc");
    config.config_path = tmp.path().join("config.toml");
    config.local_ai.runtime_enabled = true;
    let runtime = crate::inference::local_runtime_config(&config);

    let error = validated_tts_output_path(&config, &runtime, Some("/etc/openhuman-tts-test.wav"))
        .await
        .unwrap_err();

    assert!(error.contains("output_path"));
}
