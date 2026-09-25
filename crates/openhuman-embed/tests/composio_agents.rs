//! Two agents on one runtime, each with its own Composio credential, call the
//! built-in Composio tools and reach Composio with their own key only.
//!
//! One `#[test]` because a runtime claims a process-wide slot.

mod common;

use common::{
    chat_completion, offline_config, runtime, stub_backend, tool_call_completion, tool_results,
};
use openhuman_embed::{
    AgentSpec, ComposioHostCredential, Provider, Runtime, ToolGroups, Workspace,
};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY_A: &str = "ck_embed_alpha_0001";
const KEY_B: &str = "ck_embed_bravo_0002";

async fn composio_mock() -> MockServer {
    let server = MockServer::start().await;
    for (key, id, toolkit) in [(KEY_A, "ca_alpha", "gmail"), (KEY_B, "ca_bravo", "slack")] {
        Mock::given(method("GET"))
            .and(path("/v3/connected_accounts"))
            .and(header("x-api-key", key))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{ "id": id, "status": "ACTIVE", "toolkit": { "slug": toolkit } }]
            })))
            .mount(&server)
            .await;
    }
    server
}

async fn provider_calling_list_connections(reply: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(tool_call_completion("composio_list_connections", "{}")),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_completion(reply)))
        .mount(&server)
        .await;
    server
}

fn credential(key: &str, entity: &str, composio: &MockServer) -> ComposioHostCredential {
    ComposioHostCredential::direct(key)
        .entity_id(entity)
        .base_urls(
            format!("{}/v2", composio.uri()),
            format!("{}/v3", composio.uri()),
        )
}

#[test]
fn agents_call_composio_with_their_own_credential() {
    let _ = env_logger::builder().is_test(true).try_init();

    runtime().block_on(async {
        tokio::spawn(async move {
            let backend = stub_backend().await;
            let composio = composio_mock().await;
            let provider_a = provider_calling_list_connections("alpha-ok").await;
            let provider_b = provider_calling_list_connections("beta-ok").await;

            let runtime = Runtime::builder()
                .config(offline_config())
                .workspace(Workspace::Ephemeral)
                .backend_url(backend.uri())
                .api_key("th_test_key")
                .tool_groups(ToolGroups::advertised())
                .build()
                .await
                .expect("runtime builds");

            let alpha = runtime
                .agent(
                    AgentSpec::new("alpha")
                        .provider(Provider::openai_compatible(
                            format!("{}/v1", provider_a.uri()),
                            "sk-a",
                        ))
                        .composio(credential(KEY_A, "tenant-a", &composio)),
                )
                .expect("alpha instantiates");
            let beta = runtime
                .agent(
                    AgentSpec::new("beta")
                        .provider(Provider::openai_compatible(
                            format!("{}/v1", provider_b.uri()),
                            "sk-b",
                        ))
                        .composio(credential(KEY_B, "tenant-b", &composio)),
                )
                .expect("beta instantiates");

            assert_eq!(alpha.config().config_path, beta.config().config_path);
            assert_eq!(alpha.config().composio.entity_id, "tenant-a");
            assert_eq!(beta.config().composio.entity_id, "tenant-b");

            let a = alpha.run("list my connections").await.expect("alpha turn");
            assert!(a.reply.contains("alpha-ok"), "{:?}", a.reply);
            let b = beta.run("list my connections").await.expect("beta turn");
            assert!(b.reply.contains("beta-ok"), "{:?}", b.reply);

            let a_reqs = provider_a.received_requests().await.unwrap();
            let b_reqs = provider_b.received_requests().await.unwrap();
            let a_results = tool_results(&a_reqs[1]);
            let b_results = tool_results(&b_reqs[1]);
            assert!(a_results.contains("ca_alpha"), "{a_results}");
            assert!(!a_results.contains("ca_bravo"), "{a_results}");
            assert!(b_results.contains("ca_bravo"), "{b_results}");
            assert!(!b_results.contains("ca_alpha"), "{b_results}");
            for body in a_reqs.iter().chain(b_reqs.iter()) {
                let text = String::from_utf8_lossy(&body.body);
                assert!(
                    !text.contains(KEY_A) && !text.contains(KEY_B),
                    "key leaked: {text}"
                );
            }

            let keys: Vec<String> = composio
                .received_requests()
                .await
                .unwrap()
                .iter()
                .map(|r| {
                    r.headers
                        .get("x-api-key")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default()
                        .to_string()
                })
                .collect();
            assert!(keys.iter().any(|k| k == KEY_A), "{keys:?}");
            assert!(keys.iter().any(|k| k == KEY_B), "{keys:?}");
            assert!(keys.iter().all(|k| k == KEY_A || k == KEY_B), "{keys:?}");

            drop(alpha);
            drop(beta);
            drop(runtime);
        })
        .await
        .expect("library host task did not panic");
    });
}
