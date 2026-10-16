//! The reasoning-off hint and request reasoning on the managed wire.

use super::*;

#[test]
fn reasoning_hint_becomes_disabled_reasoning_on_the_managed_wire() {
    let request = apply_reasoning_hint(without_reasoning(ModelRequest::new(vec![Message::user(
        "hi",
    )])));
    assert_eq!(
        request.provider_options["reasoning"],
        serde_json::json!({ "enabled": false })
    );
}

#[test]
fn no_hint_leaves_provider_options_untouched() {
    let request = apply_reasoning_hint(ModelRequest::new(vec![Message::user("hi")]));
    assert!(request.provider_options.get("reasoning").is_none());
}

#[test]
fn request_reasoning_effort_becomes_the_managed_reasoning_object() {
    use tinyinference_llm::model::{ReasoningConfig, ReasoningEffort};
    let request = apply_reasoning_hint(
        ModelRequest::new(vec![Message::user("hi")])
            .with_reasoning(ReasoningConfig::effort(ReasoningEffort::High)),
    );
    assert_eq!(
        request.provider_options["reasoning"],
        serde_json::json!({ "effort": "high" })
    );
    assert!(
        request.reasoning.is_none(),
        "the neutral field is consumed so the transport sends no second `reasoning_effort`"
    );
}

#[test]
fn request_reasoning_none_disables_reasoning_on_the_managed_wire() {
    use tinyinference_llm::model::{ReasoningConfig, ReasoningEffort};
    let request = apply_reasoning_hint(
        ModelRequest::new(vec![Message::user("hi")])
            .with_reasoning(ReasoningConfig::effort(ReasoningEffort::None)),
    );
    assert_eq!(
        request.provider_options["reasoning"],
        serde_json::json!({ "enabled": false })
    );
}

#[test]
fn request_reasoning_budget_becomes_max_tokens() {
    use tinyinference_llm::model::{ReasoningConfig, ReasoningEffort};
    let request = apply_reasoning_hint(
        ModelRequest::new(vec![Message::user("hi")]).with_reasoning(ReasoningConfig {
            effort: Some(ReasoningEffort::High),
            budget_tokens: Some(8_000),
            summary: None,
        }),
    );
    assert_eq!(
        request.provider_options["reasoning"],
        serde_json::json!({ "max_tokens": 8000 })
    );
}

#[test]
fn suggestion_off_hint_wins_over_request_reasoning() {
    use tinyinference_llm::model::{ReasoningConfig, ReasoningEffort};
    let request = apply_reasoning_hint(
        without_reasoning(ModelRequest::new(vec![Message::user("hi")]))
            .with_reasoning(ReasoningConfig::effort(ReasoningEffort::Low)),
    );
    assert_eq!(
        request.provider_options["reasoning"],
        serde_json::json!({ "enabled": false })
    );
}

#[test]
fn explicit_reasoning_option_wins_over_the_hint() {
    let request = without_reasoning(ModelRequest::new(vec![Message::user("hi")]))
        .with_provider_options(serde_json::json!({ "reasoning": { "effort": "high" } }));
    let request = apply_reasoning_hint(request);
    assert_eq!(
        request.provider_options["reasoning"],
        serde_json::json!({ "effort": "high" })
    );
}

/// Captures the JSON body of every chat-completions request it receives.
async fn spawn_capturing_chat_server() -> (String, std::sync::Arc<std::sync::Mutex<Vec<Value>>>) {
    let bodies = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = bodies.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr").to_string();
    let app = axum::Router::new().route(
        "/openai/v1/chat/completions",
        axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(body);
                axum::Json(serde_json::json!({
                    "id": "chatcmpl-capture",
                    "object": "chat.completion",
                    "created": 1,
                    "model": "reasoning-v1",
                    "choices": [{
                        "index": 0,
                        "message": { "role": "assistant", "content": "[]" },
                        "finish_reason": "stop"
                    }],
                    "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
                }))
            }
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (addr, bodies)
}

#[tokio::test]
async fn managed_call_sends_reasoning_disabled_only_when_hinted() {
    let tmp = tempfile::TempDir::new().unwrap();
    seed_app_session(tmp.path());
    let (addr, bodies) = spawn_capturing_chat_server().await;
    let backend = backend_pointed_at(&addr, tmp.path());

    backend
        .invoke(
            &(),
            without_reasoning(ModelRequest::new(vec![Message::user("suggest")])),
        )
        .await
        .expect("hinted call");
    backend
        .invoke(&(), ModelRequest::new(vec![Message::user("chat")]))
        .await
        .expect("plain call");

    let bodies = bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2);
    assert_eq!(
        bodies[0]["reasoning"],
        serde_json::json!({ "enabled": false })
    );
    assert!(
        bodies[1].get("reasoning").is_none(),
        "an unhinted call must not change reasoning: {}",
        bodies[1]
    );
    // The hint itself never reaches the wire.
    assert!(!bodies[0].to_string().contains("openhuman_reasoning_off"));
}
