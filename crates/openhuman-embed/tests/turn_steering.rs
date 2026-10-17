//! Steering a running turn: a message added while the model is answering
//! reaches that same turn, and one the turn could not apply comes back.

mod common;

use std::time::Duration;

use common::{chat_completion, chat_requests, offline_config, route, runtime, stub_backend};
use openhuman_embed::{AgentSpec, CoreError, Runtime, Workspace};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const STEER_FRAME: &str = "[User steering message]: ";

#[test]
fn steers_reach_the_running_turn_and_leftovers_come_back() {
    runtime().block_on(async {
        tokio::spawn(scenario()).await.unwrap();
    });
}

/// A provider whose first chat call answers `draft` after `delay`, and whose
/// every later call answers `revised` at once.
async fn slow_then_fast(draft: &str, delay: Duration, revised: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(chat_completion(draft))
                .set_delay(delay),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_completion(revised)))
        .with_priority(2)
        .mount(&server)
        .await;
    server
}

/// The `(role, content)` pairs of a recorded chat request, in order.
fn transcript(request: &wiremock::Request) -> Vec<(String, String)> {
    let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| {
            let content = match &message["content"] {
                serde_json::Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            (message["role"].as_str().unwrap_or("").to_owned(), content)
        })
        .collect()
}

async fn wait_for_requests(server: &MockServer, count: usize) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while chat_requests(server).await.len() < count {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the provider should receive the chat request");
}

async fn scenario() {
    let backend = stub_backend().await;
    let runtime = Runtime::builder()
        .config(offline_config())
        .workspace(Workspace::Ephemeral)
        .backend_url(backend.uri())
        .build()
        .await
        .unwrap();

    // 1. A steer sent while the model writes its only answer of a plain chat
    //    turn: the draft stays on the transcript, the steer follows it, and the
    //    same turn calls the model again and answers with the steer in view.
    let provider = slow_then_fast(
        "Paris is 450 kilometres away.",
        Duration::from_millis(800),
        "Paris is about 280 miles away.",
    )
    .await;
    let agent = runtime
        .agent(AgentSpec::new("steered").provider(route(&provider, "test-model")))
        .unwrap();
    let mut turn = agent.turn("How far is Paris from London?");
    let steering = turn.steering_handle();
    let sent = tokio::spawn(turn.send());
    wait_for_requests(&provider, 1).await;
    steering.clone().steer("Use miles, please.").unwrap();
    let outcome = sent.await.unwrap().unwrap();

    assert_eq!(outcome.reply, "Paris is about 280 miles away.");
    assert!(outcome.unapplied_steers.is_empty());
    let requests = chat_requests(&provider).await;
    assert_eq!(requests.len(), 2, "the steer earns one more model call");
    let first = transcript(&requests[0]);
    assert!(
        !first.iter().any(|(_, text)| text.contains("Use miles")),
        "the first call ran before the steer: {first:?}"
    );
    let second = transcript(&requests[1]);
    let draft = second
        .iter()
        .position(|(role, text)| role == "assistant" && text.contains("450 kilometres"))
        .unwrap_or_else(|| panic!("the draft answer stays on the transcript: {second:?}"));
    let steer = second
        .iter()
        .position(|(role, text)| {
            role == "user" && text.contains(&format!("{STEER_FRAME}Use miles, please."))
        })
        .unwrap_or_else(|| panic!("the steer reaches the model as a user message: {second:?}"));
    assert!(draft < steer, "the steer follows the draft: {second:?}");

    // 2. Once the turn has ended, a steer is refused with its text.
    let refused = steering.steer("And in kilometres?").unwrap_err();
    assert_eq!(refused.into_text(), "And in kilometres?");
    assert!(steering.take_unapplied().is_empty());

    // 3. A steer queued before `send()` is in view of the first model call.
    let provider = slow_then_fast("Noted.", Duration::ZERO, "unexpected second call").await;
    let agent = runtime
        .agent(AgentSpec::new("primed").provider(route(&provider, "test-model")))
        .unwrap();
    let mut turn = agent.turn("Summarise the plan.");
    turn.steering_handle()
        .steer("Keep it to one line.")
        .unwrap();
    let outcome = turn.send().await.unwrap();
    assert_eq!(outcome.reply, "Noted.");
    assert!(outcome.unapplied_steers.is_empty());
    let requests = chat_requests(&provider).await;
    assert_eq!(requests.len(), 1);
    assert!(
        transcript(&requests[0])
            .iter()
            .any(|(role, text)| role == "user"
                && text.contains(&format!("{STEER_FRAME}Keep it to one line."))),
        "a primed steer reaches the first call"
    );

    // 4. A turn cancelled before it could apply a steer hands the steer back.
    let provider = slow_then_fast("too late", Duration::from_secs(4), "too late").await;
    let agent = runtime
        .agent(AgentSpec::new("cancelled").provider(route(&provider, "test-model")))
        .unwrap();
    let mut turn = agent.turn("Start something long.");
    let steering = turn.steering_handle();
    let cancel = turn.cancellation_handle();
    let sent = tokio::spawn(turn.send());
    wait_for_requests(&provider, 1).await;
    steering.steer("Actually, stop and summarise.").unwrap();
    // Let the forwarder deliver it into the run, so it must come back out.
    tokio::time::sleep(Duration::from_millis(200)).await;
    tokio::time::timeout(Duration::from_secs(5), cancel.cancel())
        .await
        .unwrap();
    assert!(matches!(
        sent.await.unwrap(),
        Err(CoreError::TurnCancelled { .. })
    ));
    assert_eq!(steering.take_unapplied(), ["Actually, stop and summarise."]);
    assert!(steering.steer("refused").is_err());
}
