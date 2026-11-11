//! Managed inference gets its default model from the registered generic
//! provider policy, while the core config remains unpinned.

mod common;

use common::{
    chat_completion, chat_requests, offline_config, runtime, stub_backend, PointedInference,
};
use openhuman_embed::{Access, AgentSpec, Runtime, Workspace};
use serde_json::Value;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn registered_provider_default_runs_with_an_unpinned_config() {
    runtime().block_on(async {
        tokio::spawn(async move {
            let backend = stub_backend().await;
            let inference = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path_regex(r"chat/completions$"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(chat_completion("fixture reply")),
                )
                .mount(&inference)
                .await;

            // The core default is intentionally captured before the provider
            // is registered. The generic registry is the only model owner.
            let config = offline_config();
            assert_eq!(config.default_model, None);
            PointedInference::install(&inference.uri());

            let runtime = Runtime::builder()
                .config(config)
                .workspace(Workspace::Ephemeral)
                .backend_url(backend.uri())
                .api_key("fixture-api-key")
                .build()
                .await
                .expect("runtime builds");
            let agent = runtime
                .agent(AgentSpec::new("managed-default").access(Access::readonly()))
                .expect("agent builds");

            let turn = agent.run("hello").await;
            if let Err(error) = &turn {
                eprintln!("managed model fixture turn error: {error:#}");
            }
            let turn = turn.expect("registered provider supplies the unpinned default model");
            assert!(turn.reply.contains("fixture reply"));

            let requests = chat_requests(&inference).await;
            assert_eq!(requests.len(), 1, "the mocked provider receives the turn");
            let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
            assert_eq!(body["model"], "embed-test-model");
        })
        .await
        .expect("test task");
    });
}
