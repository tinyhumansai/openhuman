use super::*;
use crate::config::PrivacyMode;
use crate::voice::factory::{ExternalSttProvider, SttProvider};
use tinyinference_voice::external_stt::SttApiStyle;
use wiremock::MockServer;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn standalone_saved_privacy_blocks_uploads_on_worker_threads() {
    const CHILD: &str = "OPENHUMAN_STANDALONE_PRIVACY_TEST";
    if std::env::var_os(CHILD).is_none() {
        let test = format!(
            "{}::standalone_saved_privacy_blocks_uploads_on_worker_threads",
            module_path!().split_once("::").unwrap().1
        );
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &test, "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("config.toml");
    let mock = MockServer::start().await;
    std::fs::write(&path, "[privacy]\nmode = \"local_only\"\n").unwrap();
    let mut config = Config::load_from_config_path(&path, tmp.path())
        .await
        .unwrap();
    config.api_url = Some(mock.uri());
    crate::security::credentials::api_key::store_api_key(&config, "dummy-key").unwrap();
    assert_eq!(config.privacy.mode, PrivacyMode::LocalOnly);
    assert_eq!(
        crate::security::live_policy::current_privacy_mode(),
        PrivacyMode::Standard
    );
    // Stop at the component boundary before any native recorder can load,
    // while exercising the actual standalone entry point's policy wiring.
    config.components.voice = false;
    let startup_error = run_standalone(
        &crate::core::runtime::DispatchCtx::host_default(),
        config.clone(),
        VoiceServerConfig::default(),
    )
    .await
    .unwrap_err();
    assert!(
        startup_error.contains("COMPONENT_DISABLED:voice"),
        "{startup_error}"
    );
    let endpoint = mock.uri();
    let error = tokio::spawn(async move {
        assert_eq!(
            crate::security::live_policy::current_privacy_mode(),
            PrivacyMode::LocalOnly
        );
        ExternalSttProvider::new(
            "test-speech",
            "test-model",
            endpoint,
            "dummy-key",
            SttApiStyle::OpenaiAudio,
        )
        .transcribe(&config, "AQID", Some("audio/wav"), None, None)
        .await
        .unwrap_err()
    })
    .await
    .unwrap();
    assert!(error.contains("Local-only privacy mode"), "{error}");
    assert!(mock.received_requests().await.unwrap().is_empty());
}
