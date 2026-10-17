//! A profile answers from the operator's direct chat provider and searches
//! through direct Exa and Gemini endpoints, with no managed-backend traffic.
//! One runtime and one test: SaaS operating mode is process-wide. The search
//! half needs the real module (`node scripts/modules/build.mjs tinysearch`), as in
//! `tests/search_pinned_keys_e2e.rs`; without it only chat is asserted.

mod common;

use common::{
    chat_completion, chat_requests, runtime_without_master_key, tool_names, PointedTransport,
};
use openhuman_embed::config::{
    PresentationMode, SearchConfig, SearchEngineCredentials, SearchProviderSettings, SearchRole,
};
use openhuman_embed::seams::ToolHookDecision;
use openhuman_embed::{
    DirectProfileProvider, PermissionLevel, ProfileRuntime, SaasConfig, ToolScopeSpec,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use wiremock::matchers::{header, method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// Calls every advertised search tool plus a canonical name, then finishes.
struct SearchScript;
impl Respond for SearchScript {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let has_results = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["role"] == "tool");
        let canonical = common::last_user_message(&body).contains("canonical");
        let message = if canonical && !has_results {
            let call = json!({"id":"c0","type":"function","function":{"name":"web_search_tool","arguments":"{\"query\":\"q\"}"}});
            json!({"role":"assistant","content":null,"tool_calls":[call]})
        } else if has_results || !tool_names(request).iter().any(|n| n == "web_search") {
            json!({"role":"assistant","content":"done"})
        } else {
            let calls: Vec<Value> = [
                ("web_search", json!({"query":"q"})),
                ("web_answer", json!({"query":"q"})),
                ("web_contents", json!({"urls":["https://example.com/a"]})),
            ]
            .iter()
            .enumerate()
            .map(|(i, (name, args))| json!({"id":format!("c{i}"),"type":"function","function":{"name":name,"arguments":args.to_string()}}))
            .collect();
            json!({"role":"assistant","content":null,"tool_calls":calls})
        };
        let finish = if message["tool_calls"].is_null() {
            "stop"
        } else {
            "tool_calls"
        };
        if body.get("stream").and_then(Value::as_bool) != Some(true) {
            let mut completion = chat_completion("done");
            completion["choices"][0]["message"] = message;
            completion["choices"][0]["finish_reason"] = json!(finish);
            return ResponseTemplate::new(200).set_body_json(completion);
        }
        let mut delta = message;
        if let Some(calls) = delta["tool_calls"].as_array_mut() {
            for (index, call) in calls.iter_mut().enumerate() {
                call["index"] = json!(index);
            }
        }
        let chunk = json!({"id":"f","object":"chat.completion.chunk","created":1700000000_u64,"model":"fixture-model","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
        let done = json!({"id":"f","object":"chat.completion.chunk","created":1700000000_u64,"model":"fixture-model","choices":[{"index":0,"delta":{},"finish_reason":finish}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}});
        ResponseTemplate::new(200).set_body_raw(
            format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n"),
            "text/event-stream",
        )
    }
}

#[test]
fn profile_uses_operator_direct_chat_and_search_with_no_managed_traffic() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn,openhuman_core=debug")
        .with_test_writer()
        .try_init();
    runtime_without_master_key().block_on(async {
        tokio::spawn(scenario()).await.unwrap();
    });
}

async fn scenario() {
    let module = openhuman_core::modules::local_artifact("tinysearch").is_some();
    let (managed, chat, exa, gemini) = (
        MockServer::start().await,
        MockServer::start().await,
        MockServer::start().await,
        MockServer::start().await,
    );
    Mock::given(method("POST"))
        .and(path_regex(r"/v1/chat/completions$"))
        .and(header("authorization", "Bearer operator-chat-key"))
        .respond_with(SearchScript)
        .mount(&chat)
        .await;
    for route in ["/search", "/contents"] {
        Mock::given(method("POST"))
            .and(path(route))
            .and(header("x-api-key", "operator-exa-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results":[{"title":"Exa hit","url":"https://example.com/a","text":"body"}]})))
            .mount(&exa)
            .await;
    }
    Mock::given(method("POST"))
        .and(path_regex(r"^/v1beta/models/.+:generateContent$"))
        .and(header("x-goog-api-key", "operator-gemini-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"candidates":[{"content":{"parts":[{"text":"grounded answer"}]},"groundingMetadata":{"groundingChunks":[{"web":{"uri":"https://example.com/a","title":"A"}}]}}]})))
        .mount(&gemini)
        .await;
    // Anything reaching the managed backend is a failure; it answers 500.
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(500))
        .mount(&managed)
        .await;
    PointedTransport::install(&managed.uri());

    let search = SearchConfig {
        presentation: PresentationMode::Roles,
        providers: BTreeMap::from([
            ("exa".into(), SearchProviderSettings::direct()),
            ("gemini".into(), SearchProviderSettings::direct()),
        ]),
        exa: SearchEngineCredentials {
            api_key: Some("operator-exa-key".into()),
            base_url: Some(exa.uri()),
        },
        gemini: SearchEngineCredentials {
            api_key: Some("operator-gemini-key".into()),
            base_url: Some(gemini.uri()),
        },
        ..SearchConfig::default()
    };
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
    saas.search = Some(search);
    saas.search_role_names = BTreeMap::from([
        (SearchRole::Search, "web_search".into()),
        (SearchRole::Answer, "web_answer".into()),
        (SearchRole::Contents, "web_contents".into()),
    ]);
    assert!(
        !format!("{saas:?}").contains("operator-"),
        "keys are redacted in Debug"
    );

    let runtime = ProfileRuntime::builder(saas)
        .tool_scope(ToolScopeSpec::Named(vec![
            "web_search".into(),
            "web_answer".into(),
            "web_contents".into(),
        ]))
        .can_use_tool(|_| Box::pin(async { ToolHookDecision::Proceed }))
        .channel_permissions([("web_channel", PermissionLevel::Execute)])
        .build()
        .await
        .unwrap();
    // No `set_credential`: a direct chat provider needs no backend credential.
    runtime.provision("alice").await.unwrap();
    let handle = runtime.open("alice").await.unwrap();
    let reply = handle.chat("t1", "look things up").await.unwrap();
    assert_eq!(reply.text, "done");

    let requests = chat_requests(&chat).await;
    assert!(!requests.is_empty());
    for request in &requests {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["model"], "fixture-model");
    }
    let managed_paths: Vec<_> = managed
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| format!("{} {}", r.method, r.url.path()))
        // Unrelated control-plane reads (Composio catalog, memory events) are
        // not inference or search.
        .filter(|p| {
            ["chat", "completions", "openai", "search", "gemini", "exa"]
                .iter()
                .any(|k| p.contains(k))
        })
        .collect();
    assert!(
        managed_paths.is_empty(),
        "managed inference or search traffic: {managed_paths:?}"
    );

    if module {
        let names = tool_names(&requests[0]);
        for advertised in ["web_search", "web_answer", "web_contents"] {
            assert!(
                names.iter().any(|n| n == advertised),
                "{advertised} advertised: {names:?}"
            );
        }
        assert!(
            !names.iter().any(|n| n.ends_with("_tool")),
            "canonical names hidden: {names:?}"
        );
        let exa_paths: Vec<_> = exa
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| r.url.path().to_owned())
            .collect();
        assert!(
            exa_paths.contains(&"/search".into()) && exa_paths.contains(&"/contents".into()),
            "{exa_paths:?}"
        );
        assert!(
            !gemini.received_requests().await.unwrap().is_empty(),
            "gemini answered"
        );
        let exa_before = exa.received_requests().await.unwrap().len();
        assert!(
            handle.chat("t2", "use the canonical name").await.is_err(),
            "a canonical role name is not callable"
        );
        assert_eq!(exa.received_requests().await.unwrap().len(), exa_before);
    }
}
