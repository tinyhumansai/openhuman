use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

#[tokio::test]
async fn elevenlabs_stt_uses_scribe_endpoint_request_shape_and_authentication() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/speech-to-text"))
        .and(header("xi-api-key", "test-elevenlabs-key"))
        .and(body_string_contains("name=\"model_id\""))
        .and(body_string_contains("scribe_v1"))
        .and(body_string_contains("name=\"language_code\""))
        .and(body_string_contains("en"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "text": "transcribed by scribe"
        })))
        .mount(&server)
        .await;

    let provider = ExternalSttProvider::new(
        "elevenlabs",
        "scribe_v1",
        server.uri(),
        "test-elevenlabs-key",
        SttApiStyle::ElevenLabs,
    );
    let result = provider
        .transcribe(
            &Config::default(),
            "AQID",
            Some("audio/wav"),
            Some("clip.wav"),
            Some("en"),
        )
        .await
        .expect("ElevenLabs transcription should succeed");

    assert_eq!(result.value.text, "transcribed by scribe");
    assert_eq!(result.value.provider, "elevenlabs");
}
