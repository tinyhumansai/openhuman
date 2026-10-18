//! Wire-level coverage for #7865: a `[[cloud_providers]]` entry's `headers`
//! table and the product `User-Agent` reach the provider on a chat request,
//! and the request identity does not leak onto a provider that configured
//! its own values.

use super::cloud_slug::try_create_cloud_slug_chat_model_from_string_with_native_tools;
use super::request_identity::{product_user_agent, OPENCODE_SESSION_HEADER};
use super::*;
use crate::config::schema::cloud_providers::CloudProviderCreds;
use crate::core::runtime::DispatchCtx;
use tinyinference_llm::message::Message;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn entry_for(server: &MockServer, headers: &[(&str, &str)]) -> CloudProviderCreds {
    CloudProviderCreds {
        id: "p_gateway".to_string(),
        slug: "gateway".to_string(),
        label: "Gateway".to_string(),
        endpoint: format!("{}/v1", server.uri()),
        auth_style: AuthStyle::None,
        headers: headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect(),
        ..Default::default()
    }
}

async fn mount_completion(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "gen-1",
            "model": "m",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "ok"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
        })))
        .mount(server)
        .await;
}

async fn one_request(server: &MockServer) -> wiremock::Request {
    let mut requests = server.received_requests().await.expect("recording on");
    assert_eq!(requests.len(), 1, "exactly one provider call");
    requests.remove(0)
}

fn header<'a>(request: &'a wiremock::Request, name: &str) -> Option<&'a str> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
}

async fn invoke(config: &Config) {
    let (model, model_id) = try_create_cloud_slug_chat_model_from_string_with_native_tools(
        &DispatchCtx::unbound(),
        "chat",
        "gateway:m",
        config,
        true,
        &EgressLedger::none(),
    )
    .expect("configured slug")
    .expect("model builds");
    assert_eq!(model_id, "m");
    let request = tinyinference_llm::model::ModelRequest::new(vec![Message::user("hi")]);
    let response = model.invoke(&(), request).await.expect("completion");
    assert_eq!(response.text(), "ok");
}

#[tokio::test]
async fn configured_headers_and_the_product_user_agent_reach_the_provider() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let server = MockServer::start().await;
    mount_completion(&server).await;
    let mut config = Config::default();
    config
        .cloud_providers
        .push(entry_for(&server, &[("x-tenant", "team-a")]));

    invoke(&config).await;

    let request = one_request(&server).await;
    assert_eq!(header(&request, "x-tenant"), Some("team-a"));
    assert_eq!(
        header(&request, "user-agent"),
        Some(product_user_agent().as_str())
    );
    assert_eq!(
        header(&request, OPENCODE_SESSION_HEADER),
        None,
        "a non-OpenCode endpoint gets no session header"
    );
}

#[tokio::test]
async fn configured_user_agent_and_session_headers_are_sent_verbatim() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let server = MockServer::start().await;
    mount_completion(&server).await;
    let mut config = Config::default();
    config.cloud_providers.push(entry_for(
        &server,
        &[
            ("User-Agent", "my-agent/1.0"),
            (OPENCODE_SESSION_HEADER, "ses_fixed"),
        ],
    ));

    invoke(&config).await;

    let request = one_request(&server).await;
    assert_eq!(
        request.headers.get_all("user-agent").iter().count(),
        1,
        "the configured User-Agent replaces the product one"
    );
    assert_eq!(header(&request, "user-agent"), Some("my-agent/1.0"));
    assert_eq!(header(&request, OPENCODE_SESSION_HEADER), Some("ses_fixed"));
}

#[tokio::test]
async fn an_entry_without_headers_sends_only_the_product_user_agent() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let server = MockServer::start().await;
    mount_completion(&server).await;
    let mut config = Config::default();
    config.cloud_providers.push(entry_for(&server, &[]));

    invoke(&config).await;

    let request = one_request(&server).await;
    assert_eq!(
        header(&request, "user-agent"),
        Some(product_user_agent().as_str())
    );
    assert_eq!(header(&request, "x-tenant"), None);
}
