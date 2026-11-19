//! Tests for the speech-to-text Privacy Mode chokepoint (#6870).

use super::*;
use crate::config::schema::voice_providers::{VoiceCapability, VoiceProviderCreds};
use crate::config::Config;
use crate::core::Outcome;
use crate::security::live_policy::test_privacy_scope;
use crate::voice::factory::registry::{register_stt_provider, unregister_stt_provider};
use crate::voice::factory::{create_stt_provider, SttProvider, SttResult};
use async_trait::async_trait;

const LOOPBACK: &str = "http://127.0.0.1:8080/v1";
const REMOTE: &str = "https://api.deepgram.com/v1";
const LAN: &str = "http://192.168.1.50:8080/v1";

#[test]
fn local_only_permits_an_endpoint_on_this_device() {
    for endpoint in [
        LOOPBACK,
        "http://localhost:9000/v1",
        "http://whisper.localhost/v1",
        "http://[::1]:9000/v1",
        "http://[::ffff:127.0.0.1]:9000/v1",
        "http://127.2.3.4:9000/v1",
    ] {
        assert_eq!(
            local_only_stt_violation(PrivacyMode::LocalOnly, Some(endpoint)),
            None,
            "{endpoint} is on this device and must be permitted"
        );
    }
}

#[test]
fn local_only_refuses_a_registered_provider() {
    assert_eq!(
        local_only_stt_violation(PrivacyMode::LocalOnly, None).as_deref(),
        Some(HOSTED_STT_LABEL)
    );
}

#[test]
fn local_only_refuses_remote_and_lan_endpoints_and_names_the_host() {
    for (endpoint, host) in [
        (REMOTE, "api.deepgram.com"),
        (LAN, "192.168.1.50"),
        ("http://10.0.0.7:9000/v1", "10.0.0.7"),
        ("http://localhost.example.com/v1", "localhost.example.com"),
    ] {
        assert_eq!(
            local_only_stt_violation(PrivacyMode::LocalOnly, Some(endpoint)).as_deref(),
            Some(host),
            "{endpoint} is another machine"
        );
    }
}

#[test]
fn an_unparseable_endpoint_is_refused_rather_than_assumed_local() {
    assert_eq!(
        local_only_stt_violation(PrivacyMode::LocalOnly, Some("not a url")).as_deref(),
        Some("not a url")
    );
}

#[test]
fn the_other_modes_restrict_nothing() {
    for mode in [PrivacyMode::Standard, PrivacyMode::Sensitive] {
        assert_eq!(local_only_stt_violation(mode, None), None);
        assert_eq!(local_only_stt_violation(mode, Some(REMOTE)), None);
        assert_eq!(local_only_stt_violation(mode, Some(LAN)), None);
    }
}

struct FakeStt;

#[async_trait]
impl SttProvider for FakeStt {
    fn name(&self) -> &'static str {
        "fake-privacy-stt"
    }

    async fn transcribe(
        &self,
        _config: &Config,
        _audio_base64: &str,
        _mime_type: Option<&str>,
        _file_name: Option<&str>,
        _language: Option<&str>,
    ) -> Result<Outcome<SttResult>, String> {
        unreachable!("construction-only test")
    }
}

fn with_stt_entry(endpoint: &str) -> Config {
    let mut config = Config::default();
    config.voice_providers.push(VoiceProviderCreds {
        slug: "my-whisper".into(),
        endpoint: endpoint.into(),
        capability: VoiceCapability::Stt,
        ..Default::default()
    });
    config
}

#[tokio::test]
async fn the_factory_gates_registered_providers_by_mode() {
    const SLUG: &str = "privacy-test-hosted-stt";
    register_stt_provider(SLUG, |_, _| Ok(Box::new(FakeStt)));
    let config = Config::default();

    let refused = {
        let _mode = test_privacy_scope(PrivacyMode::LocalOnly);
        create_stt_provider(SLUG, "", &config).await
    };
    let standard = {
        let _mode = test_privacy_scope(PrivacyMode::Standard);
        create_stt_provider(SLUG, "", &config).await
    };
    unregister_stt_provider(SLUG);

    let error = refused
        .err()
        .expect("LocalOnly must refuse a registered provider");
    assert!(error.to_string().contains(HOSTED_STT_LABEL), "{error}");
    assert!(error.to_string().contains("localhost"), "{error}");
    assert!(standard.is_ok(), "standard mode keeps the hosted provider");
}

#[tokio::test]
async fn the_factory_permits_a_loopback_entry_and_refuses_a_lan_one_under_local_only() {
    let _mode = test_privacy_scope(PrivacyMode::LocalOnly);

    create_stt_provider("my-whisper", "", &with_stt_entry(LOOPBACK))
        .await
        .expect("a transcription server on this machine is permitted");

    let error = create_stt_provider("my-whisper", "", &with_stt_entry(LAN))
        .await
        .err()
        .expect("a LAN server is another machine");
    assert!(error.to_string().contains("192.168.1.50"), "{error}");
}
