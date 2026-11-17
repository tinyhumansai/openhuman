//! Stateless completion cancellation drops the provider future and acknowledges it.
mod common;
use openhuman_embed::cancellation::Cancellation;
use openhuman_embed::complete::{ChatMessage, Completer, CompletionRequest};
use openhuman_embed::{CoreError, Route};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn request() -> CompletionRequest {
    CompletionRequest::new("fixture", vec![ChatMessage::user("Analyze.")])
}
async fn provider() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(common::chat_completion("ok"))
                .set_delay(Duration::from_secs(5)),
        )
        .mount(&server)
        .await;
    server
}
#[tokio::test]
async fn cancellation_is_acknowledged_after_the_attached_call_stops() {
    let server = provider().await;
    let cancel = Cancellation::default();
    let completer = Completer::new(Route::openai_compatible(
        format!("{}/v1", server.uri()),
        "fixture",
    ))
    .cancellation(cancel.clone());
    let call = tokio::spawn(async move { completer.complete(request()).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while server.received_requests().await.unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(1), cancel.cancel())
        .await
        .unwrap();
    assert!(matches!(
        call.await.unwrap(),
        Err(CoreError::Cancelled { .. })
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
#[tokio::test]
async fn a_pre_cancelled_call_makes_no_provider_request() {
    let server = provider().await;
    let cancel = Cancellation::default();
    cancel.cancel().await;
    let result = Completer::new(Route::openai_compatible(
        format!("{}/v1", server.uri()),
        "fixture",
    ))
    .cancellation(cancel)
    .complete(request())
    .await;
    assert!(matches!(result, Err(CoreError::Cancelled { .. })));
    assert!(server.received_requests().await.unwrap().is_empty());
}
#[tokio::test]
async fn deadline_refusal_is_typed_and_stops_the_provider_future() {
    let server = provider().await;
    let result = Completer::new(Route::openai_compatible(
        format!("{}/v1", server.uri()),
        "fixture",
    ))
    .timeout(Duration::from_millis(20))
    .complete(request())
    .await;
    assert!(matches!(result, Err(CoreError::DeadlineExceeded { .. })));
    // The whole-call deadline includes client construction. On a busy worker
    // it may expire before HTTP dispatch, which is a valid earlier refusal.
    assert!(server.received_requests().await.unwrap().len() <= 1);
}
