use super::*;
use openhuman_embed::__host::config::{Config, PrivacyMode};
use openhuman_embed::__host::core::{bus, events::DomainEvent};
use openhuman_embed::__host::egress::{DataKind, EgressDescriptor, EgressReason};
use openhuman_embed::__host::security::{live_policy, SecurityPolicy};
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// The production emitter publishes on the process-wide bus. Its broker belongs
// to the first tokio runtime that initializes it; a sibling test dropping that
// runtime can otherwise strand this test's receiver. Isolate each observer in a
// child process, as the existing voice processor tests do for global state.
fn isolated_event_test(name: &str) -> bool {
    const CHILD: &str = "OPENHUMAN_HOSTED_VOICE_EGRESS_TEST_CHILD";
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
    HostedStt,
    HostedTts,
}

impl Route {
    fn path(self) -> &'static str {
        match self {
            Self::HostedStt => "/openai/v1/audio/transcriptions",
            Self::HostedTts => "/openai/v1/audio/speech",
        }
    }

    fn is_stt(self) -> bool {
        matches!(self, Self::HostedStt)
    }

    fn response(self) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(match self {
            Self::HostedStt => serde_json::json!({"text":"recognized words"}),
            Self::HostedTts => serde_json::json!({"audio_base64":"AQID","audio_mime":"audio/mpeg"}),
        })
    }

    async fn call(self, config: &Config, _endpoint: &str) -> Result<String, String> {
        match self {
            Self::HostedStt => super::super::cloud_transcribe::transcribe_cloud(
                config,
                "AQID",
                &Default::default(),
            )
            .await
            .map(|outcome| outcome.value.text),
            Self::HostedTts => super::super::reply_speech::synthesize_reply(
                config,
                "private spoken words",
                &Default::default(),
            )
            .await
            .map(|outcome| outcome.value.audio_base64),
        }
    }
}

async fn check_speech_requests(mode: PrivacyMode) {
    bus::init().await.unwrap();
    // This test runs in its own child process, so the production policy can
    // be installed without changing the mode seen by parallel tests.
    let workspace = tempfile::TempDir::new().unwrap();
    live_policy::install(
        Arc::new(SecurityPolicy {
            privacy_mode: mode,
            ..SecurityPolicy::default()
        }),
        workspace.path().to_path_buf(),
        workspace.path().to_path_buf(),
    );
    let mut failures = Vec::new();
    for route in [Route::HostedStt, Route::HostedTts] {
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
        openhuman_embed::__host::security::credentials::api_key::store_api_key(
            &config,
            "test-api-key",
        )
        .unwrap();
        let mut rx = bus::BUS.get().unwrap().receiver();
        let result = route.call(&config, &server.uri()).await;
        let requests = server.received_requests().await.unwrap();
        let sentinel = format!("speech-egress-sentinel-{route:?}-{mode:?}");
        openhuman_embed::__host::egress::emit_external_transfer(
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
async fn local_only_blocks_hosted_speech_uploads() {
    if !isolated_event_test("local_only_blocks_hosted_speech_uploads") {
        return;
    }
    check_speech_requests(PrivacyMode::LocalOnly).await;
}

#[tokio::test]
async fn standard_hosted_speech_discloses_destination_and_preserves_results() {
    if !isolated_event_test("standard_hosted_speech_discloses_destination_and_preserves_results") {
        return;
    }
    check_speech_requests(PrivacyMode::Standard).await;
}

#[tokio::test]
async fn sensitive_hosted_speech_discloses_destination_and_preserves_results() {
    if !isolated_event_test("sensitive_hosted_speech_discloses_destination_and_preserves_results") {
        return;
    }
    check_speech_requests(PrivacyMode::Sensitive).await;
}

#[test]
fn invalid_hosted_speech_endpoints_do_not_echo_configuration() {
    for endpoint in [
        "not a url containing secret",
        "file:///private/secret",
        "mailto:secret@example.test",
    ] {
        let error =
            disclose_speech_transfer("hosted-test", endpoint, DataKind::Prompt).unwrap_err();
        assert!(error.contains("speech provider endpoint"));
        assert!(!error.contains("secret"));
    }
}
