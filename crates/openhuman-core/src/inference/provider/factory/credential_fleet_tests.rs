use super::*;
use tinyinference_llm::message::{ImageRef, UserMessage};
#[test]
fn image_endpoints_only_apply_when_request_contains_images() {
    let text = ModelRequest {
        messages: vec![Message::user("text")],
        ..Default::default()
    };
    assert!(!request_has_image(&text));
    let image = ModelRequest {
        messages: vec![Message::User(UserMessage {
            content: vec![
                ContentBlock::Text("What's here?".into()),
                ContentBlock::Image(ImageRef {
                    url: "data:image/png;base64,AAAA".into(),
                    mime_type: Some("image/png".into()),
                }),
            ],
        })],
        ..Default::default()
    };
    assert!(request_has_image(&image));
}
struct CapabilityStubModel {
    modality: InputModality,
}
#[async_trait]
impl ChatModel<()> for CapabilityStubModel {
    fn supports_input(&self, modality: InputModality, _: &str, _: InputSource) -> bool {
        modality == self.modality
    }

    async fn invoke(&self, _: &(), _: ModelRequest) -> tinyinference_llm::Result<ModelResponse> {
        Err(tinyinference_llm::Error::Model(
            "capability probe only".into(),
        ))
    }
}

#[test]
fn fleet_reports_non_image_capabilities_of_the_model_that_receives_the_request() {
    let fleet = CredentialFleetModel::new(
        "fleet-capability-route".into(),
        vec!["credential".into()],
        vec![Arc::new(CapabilityStubModel {
            modality: InputModality::Audio,
        })],
        Some(vec![Arc::new(CapabilityStubModel {
            modality: InputModality::Image,
        })]),
        RotationStrategy::RoundRobin,
    );

    assert!(fleet.supports_input(InputModality::Image, "image/png", InputSource::Base64));
    assert!(fleet.supports_input(InputModality::Audio, "audio/wav", InputSource::Url));
    assert!(!fleet.supports_input(InputModality::Document, "application/pdf", InputSource::Url));
    assert!(!fleet.supports_input(InputModality::Video, "video/mp4", InputSource::Url));
}

struct StubStreamModel {
    fail_auth: bool,
}
#[async_trait]
impl ChatModel<()> for StubStreamModel {
    async fn invoke(&self, _: &(), _: ModelRequest) -> tinyinference_llm::Result<ModelResponse> {
        Err(tinyinference_llm::Error::Model(
            "not used by stream test".into(),
        ))
    }
    async fn stream(&self, _: &(), _: ModelRequest) -> tinyinference_llm::Result<ModelStream> {
        let events = if self.fail_auth {
            vec![
                ModelStreamItem::Started,
                ModelStreamItem::ProviderFailed(ProviderError {
                    status: Some(429),
                    ..Default::default()
                }),
            ]
        } else {
            vec![
                ModelStreamItem::Started,
                ModelStreamItem::MessageDelta(tinyinference_llm::message::MessageDelta {
                    text: "selected-second-key".into(),
                    reasoning: String::new(),
                    tool_call: None,
                }),
                ModelStreamItem::Failed("test stream finished".into()),
            ]
        };
        Ok(ModelStream::new(Box::pin(futures::stream::iter(events))))
    }
}
#[tokio::test]
async fn pre_content_rate_limit_rotates_stream_before_exposing_error() {
    use futures::StreamExt;
    let scope = "fleet-stream-first-429";
    let fleet = CredentialFleetModel::new(
        scope.into(),
        vec!["first".into(), "second".into()],
        vec![
            Arc::new(StubStreamModel { fail_auth: true }),
            Arc::new(StubStreamModel { fail_auth: false }),
        ],
        None,
        RotationStrategy::RoundRobin,
    );
    let stream = fleet.stream(&(), ModelRequest::default()).await.unwrap();
    let events: Vec<ModelStreamItem> = stream.collect().await;
    assert!(matches!(
        events.get(1),
        Some(ModelStreamItem::MessageDelta(_))
    ));
    assert_eq!(key_pool::snapshot(scope)[0].status, "cooldown");
}

#[test]
fn deferred_route_is_keyed_to_credential_not_array_position() {
    let mut handle = tinyinference_llm::model::DeferredHandle::new("openai", "resp_123");
    stamp_deferred_handle(&mut handle, "secret-one", true);
    assert!(serde_json::to_string(&handle)
        .unwrap()
        .find("secret-one")
        .is_none());
    let route = decode_deferred_handle(&handle).unwrap();
    assert_eq!(route.key_fingerprint, credential_fingerprint("secret-one"));
    assert!(route.image);
}

#[test]
fn provider_failures_are_rerouted_conservatively() {
    assert_eq!(
        classify(Some(401), None, None).unwrap().failure,
        KeyFailure::Authentication
    );
    assert_eq!(
        classify(Some(429), None, Some(5000)).unwrap().failure,
        KeyFailure::RateLimited
    );
    assert_eq!(
        classify(Some(403), Some("quota_exhausted"), None)
            .unwrap()
            .failure,
        KeyFailure::RateLimited
    );
    assert!(classify(Some(400), None, None).is_none());
    assert!(classify(Some(403), Some("forbidden"), None).is_none());
    // A provider can use HTTP 403 for an invalid model or request. A broad
    // substring match for "invalid" must NOT evict an otherwise healthy API key.
    for code in [
        "invalid_model",
        "invalid_model_id",
        "invalid_request",
        "invalid_request_error",
        "invalid_parameter",
        "unauthorized_model",
        "quota_configuration_error",
        "rate_limit_policy_disabled",
    ] {
        assert!(
            classify(Some(403), Some(code), None).is_none(),
            "403 {code} is not evidence that another credential would succeed"
        );
    }
    for code in [
        "invalid_api_key",
        "api_key_invalid",
        "invalid_credentials",
        "authentication_error",
        "unauthorized",
        "api_key_revoked",
        "token_revoked",
    ] {
        assert_eq!(
            classify(Some(403), Some(code), None).unwrap().failure,
            KeyFailure::Authentication,
            "{code} should evict only the rejected credential"
        );
    }
    for code in [
        "quota_exhausted",
        "insufficient_quota",
        "resource_exhausted",
        "rate_limit_exceeded",
    ] {
        assert_eq!(
            classify(Some(403), Some(code), None).unwrap().failure,
            KeyFailure::RateLimited,
            "{code} is a credential-scoped cooldown"
        );
    }
}

#[test]
fn typed_transient_failures_can_try_another_key_without_marking_it_invalid() {
    for status in [None, Some(408), Some(503)] {
        let error = ProviderError {
            status,
            retryable: true,
            ..Default::default()
        };
        assert_eq!(
            provider_decision(&error).unwrap().failure,
            KeyFailure::Other
        );
    }
    let permanent = ProviderError {
        status: Some(503),
        retryable: false,
        ..Default::default()
    };
    assert!(provider_decision(&permanent).is_none());
}

#[test]
fn a_partial_answer_is_never_rerouted_to_another_credential() {
    let error = ProviderError {
        status: Some(429),
        retryable: true,
        partial_message: Some(ModelResponse::assistant("partial answer").message),
        ..Default::default()
    };
    assert!(provider_decision(&error).is_none());
}

struct CountingForbiddenModel {
    attempts: Arc<std::sync::atomic::AtomicUsize>,
}
#[async_trait]
impl ChatModel<()> for CountingForbiddenModel {
    async fn invoke(&self, _: &(), _: ModelRequest) -> tinyinference_llm::Result<ModelResponse> {
        self.attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(tinyinference_llm::Error::Provider(Box::new(
            ProviderError {
                status: Some(403),
                code: Some("invalid_model".into()),
                ..Default::default()
            },
        )))
    }

    async fn stream(&self, _: &(), _: ModelRequest) -> tinyinference_llm::Result<ModelStream> {
        self.attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(ModelStream::new(Box::pin(futures::stream::iter(vec![
            ModelStreamItem::Started,
            ModelStreamItem::ProviderFailed(ProviderError {
                status: Some(403),
                code: Some("invalid_model".into()),
                ..Default::default()
            }),
        ]))))
    }
}
#[tokio::test]
async fn invalid_model_403_does_not_evict_api_keys_or_retry_another_credential() {
    use futures::StreamExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let first = Arc::new(AtomicUsize::new(0));
    let second = Arc::new(AtomicUsize::new(0));
    let scope = "fleet-invalid-model-403-unary";
    let fleet = CredentialFleetModel::new(
        scope.into(),
        vec!["unary-first".into(), "unary-second".into()],
        vec![
            Arc::new(CountingForbiddenModel {
                attempts: Arc::clone(&first),
            }),
            Arc::new(CountingForbiddenModel {
                attempts: Arc::clone(&second),
            }),
        ],
        None,
        RotationStrategy::RoundRobin,
    );
    assert!(fleet.invoke(&(), ModelRequest::default()).await.is_err());
    assert_eq!(first.load(Ordering::SeqCst), 1);
    assert_eq!(second.load(Ordering::SeqCst), 0);
    assert_eq!(key_pool::snapshot(scope)[0].status, "healthy");
    key_pool::forget(scope);

    let first = Arc::new(AtomicUsize::new(0));
    let second = Arc::new(AtomicUsize::new(0));
    let scope = "fleet-invalid-model-403-stream";
    let fleet = CredentialFleetModel::new(
        scope.into(),
        vec!["stream-first".into(), "stream-second".into()],
        vec![
            Arc::new(CountingForbiddenModel {
                attempts: Arc::clone(&first),
            }),
            Arc::new(CountingForbiddenModel {
                attempts: Arc::clone(&second),
            }),
        ],
        None,
        RotationStrategy::RoundRobin,
    );
    let stream = fleet.stream(&(), ModelRequest::default()).await.unwrap();
    let output: Vec<ModelStreamItem> = stream.collect().await;
    assert_eq!(first.load(Ordering::SeqCst), 1);
    assert_eq!(second.load(Ordering::SeqCst), 0);
    assert!(output
        .iter()
        .any(|event| matches!(event, ModelStreamItem::ProviderFailed(_))));
    assert_eq!(key_pool::snapshot(scope)[0].status, "healthy");
    key_pool::forget(scope);
}

struct PostOutputRateLimitModel {
    attempts: Arc<std::sync::atomic::AtomicUsize>,
}
#[async_trait]
impl ChatModel<()> for PostOutputRateLimitModel {
    async fn invoke(&self, _: &(), _: ModelRequest) -> tinyinference_llm::Result<ModelResponse> {
        Err(tinyinference_llm::Error::Model("stream only".into()))
    }
    async fn stream(&self, _: &(), _: ModelRequest) -> tinyinference_llm::Result<ModelStream> {
        self.attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(ModelStream::new(Box::pin(futures::stream::iter(vec![
            ModelStreamItem::Started,
            ModelStreamItem::MessageDelta(tinyinference_llm::message::MessageDelta::text(
                "visible output",
            )),
            ModelStreamItem::ProviderFailed(ProviderError {
                status: Some(429),
                ..Default::default()
            }),
        ]))))
    }
}

#[tokio::test]
async fn post_content_upstream_429_cannot_replay_or_duplicate_visible_tool_output() {
    use futures::StreamExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let first = Arc::new(AtomicUsize::new(0));
    let second = Arc::new(AtomicUsize::new(0));
    let scope = "fleet-postcontent-no-replay";
    let fleet = CredentialFleetModel::new(
        scope.into(),
        vec!["already-streamed".into(), "unused-backup".into()],
        vec![
            Arc::new(PostOutputRateLimitModel {
                attempts: Arc::clone(&first),
            }),
            Arc::new(PostOutputRateLimitModel {
                attempts: Arc::clone(&second),
            }),
        ],
        None,
        RotationStrategy::RoundRobin,
    );
    let events: Vec<ModelStreamItem> = fleet
        .stream(&(), ModelRequest::default())
        .await
        .unwrap()
        .collect()
        .await;
    assert_eq!(first.load(Ordering::SeqCst), 1);
    assert_eq!(second.load(Ordering::SeqCst), 0);
    assert_eq!(
        events
            .iter()
            .filter(|item| matches!(item, ModelStreamItem::MessageDelta(_)))
            .count(),
        1,
        "an emitted delta is never replayed across credentials"
    );
    assert!(
        matches!(events.last(), Some(ModelStreamItem::ProviderFailed(_))),
        "the upstream failure must remain visible"
    );
    assert_eq!(key_pool::snapshot(scope)[0].status, "cooldown");
    assert!(key_pool::snapshot(scope)
        .iter()
        .all(|entry| entry.active_leases == 0));
    key_pool::forget(scope);
}

struct CountingRateLimitedModel {
    attempts: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl ChatModel<()> for CountingRateLimitedModel {
    async fn invoke(&self, _: &(), _: ModelRequest) -> tinyinference_llm::Result<ModelResponse> {
        self.attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(tinyinference_llm::Error::Provider(Box::new(
            ProviderError {
                status: Some(429),
                ..Default::default()
            },
        )))
    }

    async fn stream(&self, _: &(), _: ModelRequest) -> tinyinference_llm::Result<ModelStream> {
        self.attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(ModelStream::new(Box::pin(futures::stream::iter(vec![
            ModelStreamItem::Started,
            ModelStreamItem::ProviderFailed(ProviderError {
                status: Some(429),
                ..Default::default()
            }),
        ]))))
    }
}

#[tokio::test]
async fn fleet_fails_over_at_most_the_per_call_budget_for_unary_requests() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let scope = "fleet-bounded-429-unary";
    let attempts = Arc::new(AtomicUsize::new(0));
    let keys: Vec<String> = (0..8).map(|index| format!("unary-key-{index}")).collect();
    let models: Vec<Arc<dyn ChatModel<()>>> = keys
        .iter()
        .map(|_| {
            Arc::new(CountingRateLimitedModel {
                attempts: Arc::clone(&attempts),
            }) as Arc<dyn ChatModel<()>>
        })
        .collect();
    let fleet = CredentialFleetModel::new(
        scope.into(),
        keys,
        models,
        None,
        RotationStrategy::RoundRobin,
    );
    assert!(fleet.invoke(&(), ModelRequest::default()).await.is_err());
    assert_eq!(attempts.load(Ordering::SeqCst), MAX_KEY_ATTEMPTS_PER_CALL);
    key_pool::forget(scope);
}

#[tokio::test]
async fn fleet_fails_over_at_most_the_per_call_budget_for_precontent_stream_failures() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let scope = "fleet-bounded-429-stream";
    let attempts = Arc::new(AtomicUsize::new(0));
    let keys: Vec<String> = (0..8).map(|index| format!("stream-key-{index}")).collect();
    let models: Vec<Arc<dyn ChatModel<()>>> = keys
        .iter()
        .map(|_| {
            Arc::new(CountingRateLimitedModel {
                attempts: Arc::clone(&attempts),
            }) as Arc<dyn ChatModel<()>>
        })
        .collect();
    let fleet = CredentialFleetModel::new(
        scope.into(),
        keys,
        models,
        None,
        RotationStrategy::RoundRobin,
    );
    assert!(fleet.stream(&(), ModelRequest::default()).await.is_err());
    assert_eq!(attempts.load(Ordering::SeqCst), MAX_KEY_ATTEMPTS_PER_CALL);
    key_pool::forget(scope);
}
