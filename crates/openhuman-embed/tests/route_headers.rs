//! Per-route attribution headers follow only the endpoint and turn named by the host.

mod common;

use common::{chat_requests, offline_config, provider, route, runtime, stub_backend};
use openhuman_embed::{AgentSpec, Provider, ReasoningEffort, Route, Runtime, Workspace};

#[test]
fn attribution_headers_are_scoped_to_each_agent_and_turn_route() {
    runtime().block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let backend = stub_backend().await;
    let (a_provider, b_provider) = tokio::join!(provider("a"), provider("b"));
    let runtime = Runtime::builder()
        .config(offline_config())
        .workspace(Workspace::Ephemeral)
        .backend_url(backend.uri())
        .build()
        .await
        .unwrap();
    let a_route = Route::openai_compatible(format!("{}/v1", a_provider.uri()), "a-key")
        .header("x-medulla-agent-id", "worker-a")
        .header("x-medulla-run-id", "a-run")
        .provider_options(serde_json::json!({
            "provider": {"sort": "latency"},
            "usage": {"include": false}
        }))
        .reasoning_effort(ReasoningEffort::High);
    assert!(
        !format!("{a_route:?}").contains("a-run"),
        "header values must stay out of Debug"
    );
    let a = runtime
        .agent(AgentSpec::new("worker-a").provider(route(&a_provider, "fixture")))
        .unwrap();
    let b = runtime
        .agent(
            AgentSpec::new("worker-b").provider(
                Provider::routed(
                    Route::openai_compatible(format!("{}/v1", b_provider.uri()), "b-key")
                        .header("x-medulla-agent-id", "worker-b"),
                )
                .model("fixture"),
            ),
        )
        .unwrap();
    let (a_result, b_result) = tokio::join!(
        a.turn("a")
            .route(a_route)
            .provider_options(serde_json::json!({"usage": {"include": true}}))
            .send(),
        b.run("b")
    );
    a_result.unwrap();
    b_result.unwrap();
    // The next turn inherits the agent provider, not the previous turn's headers.
    a.run("next a").await.unwrap();
    let a_requests = chat_requests(&a_provider).await;
    let b_requests = chat_requests(&b_provider).await;
    assert_eq!(a_requests.len(), 2);
    assert_eq!(b_requests.len(), 1);
    assert_eq!(
        a_requests[0].headers.get("x-medulla-agent-id").unwrap(),
        "worker-a"
    );
    assert_eq!(
        a_requests[0].headers.get("x-medulla-run-id").unwrap(),
        "a-run"
    );
    assert_eq!(
        a_requests[0].headers.get("authorization").unwrap(),
        "Bearer a-key"
    );
    let a_body: serde_json::Value = serde_json::from_slice(&a_requests[0].body).unwrap();
    assert_eq!(a_body["provider"], serde_json::json!({"sort": "latency"}));
    assert_eq!(a_body["usage"], serde_json::json!({"include": true}));
    assert_eq!(a_body["reasoning_effort"], "high");
    assert!(a_requests[1].headers.get("x-medulla-run-id").is_none());
    assert_eq!(
        b_requests[0].headers.get("x-medulla-agent-id").unwrap(),
        "worker-b"
    );
    assert!(b_requests[0].headers.get("x-medulla-run-id").is_none());
    assert!(backend
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.headers.get("x-medulla-run-id").is_none()));
}
