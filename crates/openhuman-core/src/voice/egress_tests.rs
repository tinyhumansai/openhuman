use super::*;
use crate::config::{Config, PrivacyMode};
use crate::core::events::DomainEvent;
use crate::security::egress::{DataKind, EgressDescriptor, EgressReason};
use crate::security::live_policy::test_privacy_scope;
use crate::voice::factory::{ExternalSttProvider, ExternalTtsProvider, SttProvider, TtsProvider};
use tinyinference_voice::{external_stt::SttApiStyle, external_tts::TtsApiStyle};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// The production emitter publishes on the process-wide bus. Its broker belongs
// to the first tokio runtime that initializes it; a sibling test dropping that
// runtime can otherwise strand this test's receiver. Isolate each observer in a
// child process, as the existing voice processor tests do for global state.
fn isolated_event_test(name: &str) -> bool {
    const CHILD: &str = "OPENHUMAN_VOICE_EGRESS_TEST_CHILD";
    let module = module_path!().split_once("::").expect("crate module").1;
    let test = format!("{module}::{name}");
    if std::env::var(CHILD).as_deref() == Ok(test.as_str()) {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", &test, "--nocapture", "--test-threads=1"])
        .env(CHILD, &test)
        .output()
        .expect("run isolated speech egress test");
    assert!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "isolated test {test} failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

#[derive(Clone, Copy, Debug)]
enum Route {
    OpenaiStt,
    DeepgramStt,
    ElevenlabsStt,
    OpenaiTts,
    ElevenlabsTts,
}

impl Route {
    fn path(self) -> &'static str {
        match self {
            Self::OpenaiStt => "/audio/transcriptions",
            Self::DeepgramStt => "/listen",
            Self::ElevenlabsStt => "/speech-to-text",
            Self::OpenaiTts => "/audio/speech",
            Self::ElevenlabsTts => "/text-to-speech/test-voice",
        }
    }

    fn is_stt(self) -> bool {
        matches!(
            self,
            Self::OpenaiStt | Self::DeepgramStt | Self::ElevenlabsStt
        )
    }

    fn response(self) -> ResponseTemplate {
        let json = match self {
            Self::DeepgramStt => {
                serde_json::json!({"results":{"channels":[{"alternatives":[{"transcript":"recognized words"}]}]}})
            }
            Self::OpenaiStt | Self::ElevenlabsStt => {
                serde_json::json!({"text":"recognized words"})
            }
            _ => {
                return ResponseTemplate::new(200)
                    .insert_header("content-type", "audio/mpeg")
                    .set_body_bytes(vec![1, 2, 3])
            }
        };
        ResponseTemplate::new(200).set_body_json(json)
    }

    async fn call(self, config: &Config, endpoint: &str) -> Result<String, String> {
        match self {
            Self::OpenaiStt | Self::DeepgramStt | Self::ElevenlabsStt => {
                let style = match self {
                    Self::DeepgramStt => SttApiStyle::Deepgram,
                    Self::ElevenlabsStt => SttApiStyle::ElevenLabs,
                    _ => SttApiStyle::OpenaiAudio,
                };
                ExternalSttProvider::new("speech-test", "test-model", endpoint, "test-key", style)
                    .transcribe(
                        config,
                        "AQID",
                        Some("audio/wav"),
                        Some("private-recording.wav"),
                        None,
                    )
                    .await
                    .map(|o| o.value.text)
            }
            Self::OpenaiTts | Self::ElevenlabsTts => {
                let style = if matches!(self, Self::OpenaiTts) {
                    TtsApiStyle::OpenaiAudio
                } else {
                    TtsApiStyle::ElevenLabs
                };
                ExternalTtsProvider::new("speech-test", "test-voice", endpoint, "test-key", style)
                    .synthesize(config, "private spoken words", None)
                    .await
                    .map(|o| o.value.audio_base64)
            }
        }
    }
}

async fn check_speech_requests(mode: PrivacyMode) {
    crate::core::bus::init().await.unwrap();
    let _mode = test_privacy_scope(mode);
    let mut failures = Vec::new();
    for route in [
        Route::OpenaiStt,
        Route::DeepgramStt,
        Route::ElevenlabsStt,
        Route::OpenaiTts,
        Route::ElevenlabsTts,
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(route.path()))
            .respond_with(route.response())
            .mount(&server)
            .await;
        let tmp = tempfile::TempDir::new().unwrap();
        let config = Config {
            workspace_dir: tmp.path().join("workspace"),
            action_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            api_url: Some(server.uri()),
            ..Config::default()
        };
        crate::security::credentials::api_key::store_api_key(&config, "test-api-key").unwrap();
        let mut rx = crate::core::bus::BUS.get().unwrap().receiver();
        let result = route.call(&config, &server.uri()).await;
        let requests = server.received_requests().await.unwrap();
        let sentinel = format!("speech-egress-sentinel-{route:?}-{mode:?}");
        crate::security::egress::emit_external_transfer(
            None,
            EgressDescriptor::network_fetch(&sentinel),
        );
        let mut transfers = Vec::new();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(DomainEvent::ExternalTransferPending { descriptor, .. }) =
                    rx.recv().await
                {
                    if descriptor.service == sentinel {
                        break;
                    }
                    if descriptor
                        .service
                        .contains(&server.address().port().to_string())
                    {
                        transfers.push(descriptor);
                    }
                }
            }
        })
        .await
        .expect("disclosure sentinel should arrive");
        if mode == PrivacyMode::LocalOnly {
            if !result
                .as_ref()
                .is_err_and(|err| err.contains("Local-only privacy mode"))
                || !requests.is_empty()
                || !transfers.is_empty()
            {
                failures.push(format!(
                    "{route:?}: result={result:?}, requests={}, disclosures={}",
                    requests.len(),
                    transfers.len()
                ));
            }
        } else {
            assert_eq!(
                result.unwrap_or_else(|e| panic!("{route:?}: {e}")),
                if route.is_stt() {
                    "recognized words"
                } else {
                    "AQID"
                }
            );
            assert_eq!(
                requests.len(),
                1,
                "{route:?}: permitted request must reach mock"
            );
            assert_eq!(
                transfers.len(),
                1,
                "{route:?}: permitted request must disclose its actual destination"
            );
            let desc = &transfers[0];
            assert_eq!(desc.service, server.uri());
            assert!(desc.is_external);
            assert_eq!(desc.reason, EgressReason::Inference);
            assert_eq!(
                desc.data_kinds,
                vec![if route.is_stt() {
                    DataKind::FileContent
                } else {
                    DataKind::Prompt
                }]
            );
            let serialized = serde_json::to_string(desc).unwrap();
            for private in [
                "private spoken words",
                "private-recording.wav",
                "test-api-key",
                "test-key",
                "AQID",
            ] {
                assert!(
                    !serialized.contains(private),
                    "disclosure must not include payloads or credentials"
                );
            }
        }
    }
    assert!(
        failures.is_empty(),
        "Local-only must block every speech route before upload:\n{}",
        failures.join("\n")
    );
}

#[tokio::test]
async fn local_only_blocks_all_external_speech_uploads() {
    if !isolated_event_test("local_only_blocks_all_external_speech_uploads") {
        return;
    }
    check_speech_requests(PrivacyMode::LocalOnly).await;
}

#[tokio::test]
async fn standard_speech_requests_disclose_destination_and_preserve_results() {
    if !isolated_event_test("standard_speech_requests_disclose_destination_and_preserve_results") {
        return;
    }
    check_speech_requests(PrivacyMode::Standard).await;
}

#[tokio::test]
async fn sensitive_speech_requests_disclose_destination_and_preserve_results() {
    if !isolated_event_test("sensitive_speech_requests_disclose_destination_and_preserve_results") {
        return;
    }
    check_speech_requests(PrivacyMode::Sensitive).await;
}

#[tokio::test]
async fn speech_disclosure_strips_endpoint_credentials_paths_and_queries() {
    if !isolated_event_test("speech_disclosure_strips_endpoint_credentials_paths_and_queries") {
        return;
    }
    crate::core::bus::init().await.unwrap();
    let _mode = test_privacy_scope(PrivacyMode::Standard);
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();
    disclose_speech_transfer(
        "sanitized-speech-test",
        "https://user:secret@speech.example.test:8443/private/path?api_key=hidden#fragment",
        DataKind::FileContent,
    )
    .unwrap();
    let descriptor = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(DomainEvent::ExternalTransferPending { descriptor, .. }) = rx.recv().await {
                if descriptor.provider_slug == "sanitized-speech-test" {
                    break descriptor;
                }
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(descriptor.service, "https://speech.example.test:8443");
    let serialized = serde_json::to_string(&descriptor).unwrap();
    for secret in [
        "user:",
        "secret",
        "private/path",
        "api_key",
        "hidden",
        "fragment",
    ] {
        assert!(!serialized.contains(secret));
    }
}

#[test]
fn invalid_speech_endpoints_fail_without_echoing_configuration() {
    let _mode = test_privacy_scope(PrivacyMode::Standard);
    for endpoint in [
        "not a url containing secret",
        "file:///private/secret",
        "mailto:secret@example.test",
    ] {
        let err = disclose_speech_transfer("speech-test", endpoint, DataKind::Prompt).unwrap_err();
        assert!(err.contains("speech provider endpoint"));
        assert!(!err.contains("secret"));
    }
}

#[tokio::test]
async fn external_speech_redirects_never_replay_content_to_another_origin() {
    if !isolated_event_test("external_speech_redirects_never_replay_content_to_another_origin") {
        return;
    }
    crate::core::bus::init().await.unwrap();
    let _mode = test_privacy_scope(PrivacyMode::Standard);
    for status in [307, 308] {
        for route in [
            Route::OpenaiStt,
            Route::DeepgramStt,
            Route::ElevenlabsStt,
            Route::OpenaiTts,
            Route::ElevenlabsTts,
        ] {
            let source = MockServer::start().await;
            let target = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path(route.path()))
                .respond_with(
                    ResponseTemplate::new(status)
                        .insert_header("location", format!("{}{}", target.uri(), route.path())),
                )
                .mount(&source)
                .await;
            Mock::given(method("POST"))
                .and(path(route.path()))
                .respond_with(route.response())
                .mount(&target)
                .await;
            let result = route.call(&Config::default(), &source.uri()).await;
            assert!(
                result.is_err(),
                "{route:?} accepted redirect {status}: {result:?}"
            );
            assert_eq!(source.received_requests().await.unwrap().len(), 1);
            assert!(
                target.received_requests().await.unwrap().is_empty(),
                "{route:?} replayed content after {status}"
            );
        }
    }
}
