//! The operator's system prompt is the whole system prompt of a profile's
//! turns: the model sees exactly that text, with no SOUL/IDENTITY content,
//! routing or tool guidance around it. One runtime and one test: SaaS
//! operating mode is process-wide.

mod common;

use common::{chat_completion, chat_requests, runtime_without_master_key, PointedInference};
use openhuman_embed::profiles::ProfileCredentialKind;
use openhuman_embed::{DirectProfileProvider, ProfileRuntime, SaasConfig};
use serde_json::Value;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PERSONA: &str = "You are Chottu, a friendly chat companion. Reply in one short sentence.";

#[test]
fn operator_system_prompt_is_the_only_system_prompt_a_profile_sees() {
    runtime_without_master_key().block_on(async {
        tokio::spawn(scenario()).await.unwrap();
    });
}

async fn scenario() {
    let (managed, chat) = (MockServer::start().await, MockServer::start().await);
    Mock::given(method("POST"))
        .and(path_regex(r"/v1/chat/completions$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_completion("hi")))
        .mount(&chat)
        .await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(500))
        .mount(&managed)
        .await;
    PointedInference::install(&managed.uri());

    let root = tempfile::tempdir().unwrap();
    let mut saas = SaasConfig::new(root.path());
    saas.chat_provider = Some(
        DirectProfileProvider::openai_compatible(
            format!("{}/v1", chat.uri()),
            "operator-chat-key",
            "fixture-model",
        )
        .unwrap(),
    );
    let empty = ProfileRuntime::builder(saas.clone()).system_prompt("  ");
    assert!(
        empty.build().await.is_err(),
        "an empty system prompt is refused at build time"
    );

    let runtime = ProfileRuntime::builder(saas)
        .system_prompt(PERSONA)
        .build()
        .await
        .unwrap();
    let provisioned = runtime.provision("alice").await.unwrap();
    runtime
        .set_credential(
            &provisioned.profile_id,
            ProfileCredentialKind::ApiKey,
            "fixture-key",
        )
        .await
        .unwrap();
    let handle = runtime.open("alice").await.unwrap();
    assert_eq!(handle.chat("t1", "hello").await.unwrap().text, "hi");

    let requests = chat_requests(&chat).await;
    assert!(!requests.is_empty());
    for request in &requests {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let system: Vec<&str> = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "system")
            .filter_map(|m| m["content"].as_str())
            .collect();
        assert_eq!(system, [PERSONA], "the persona is the whole system prompt");
    }
}
