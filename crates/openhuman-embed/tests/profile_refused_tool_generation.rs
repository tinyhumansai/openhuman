//! A refused tool call must not rewrite an earlier turn's result. Providers
//! reuse tool-call ids across turns (`call_1` again), so a failure in this turn
//! used to mark the earlier turn's successful `cron` result as failed; the
//! committed rows then no longer extended the persisted transcript, and the
//! session opened a successor generation (which collides when a stale session
//! does it again). One runtime and one test: SaaS operating mode is
//! process-wide.

mod common;

use std::sync::Arc;

use common::{chat_completion, chat_requests, runtime_without_master_key, PointedInference};
use openhuman_embed::seams::ToolHookDecision;
use openhuman_embed::{
    DirectProfileProvider, PermissionLevel, ProfileRuntime, SaasConfig, SessionStoreProvider,
    ToolScopeSpec,
};
use serde_json::{json, Value};
use tinyagents_session::port::InMemorySessionStores;
use wiremock::matchers::{header, method, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// Scripts the model from the last user message.
struct Script {
    fire_at: String,
}

impl Respond for Script {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let messages = body["messages"].as_array().unwrap();
        let last_user = messages.iter().rposition(|m| m["role"] == "user").unwrap();
        let results = messages[last_user + 1..]
            .iter()
            .filter(|m| m["role"] == "tool")
            .count();
        let prompt = common::last_user_message(&body);
        let call = |name: &str, args: Value| {
            let mut completion = common::tool_call_completion(name, &args.to_string());
            completion["choices"][0]["message"]["tool_calls"][0]["id"] =
                json!(format!("call_{results}"));
            completion
        };
        let completion = if prompt.contains("reminder ping") {
            chat_completion("pong")
        } else if prompt.contains("schedule it") && results == 0 {
            call("tool_search", json!({"query":"cron schedule job"}))
        } else if prompt.contains("schedule it") && results == 1 {
            call(
                "cron",
                json!({"action":"add","job_type":"agent","name":"ping",
                    "schedule":{"kind":"at","at":self.fire_at},"prompt":"reminder ping"}),
            )
        } else if prompt.contains("shell job") && results == 0 {
            call("tool_search", json!({"query":"cron schedule job"}))
        } else if prompt.contains("shell job") && results == 1 {
            call(
                "cron",
                json!({"action":"add","job_type":"shell","name":"sh",
                    "schedule":{"kind":"at","at":self.fire_at},"command":"echo hi"}),
            )
        } else {
            chat_completion("done")
        };
        if body.get("stream").and_then(Value::as_bool) != Some(true) {
            return ResponseTemplate::new(200).set_body_json(completion);
        }
        let mut delta = completion["choices"][0]["message"].clone();
        if let Some(calls) = delta["tool_calls"].as_array_mut() {
            for (index, call) in calls.iter_mut().enumerate() {
                call["index"] = json!(index);
            }
        }
        let chunk = json!({"id":"f","object":"chat.completion.chunk","created":1700000000_u64,"model":"fixture-model","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
        let done = json!({"id":"f","object":"chat.completion.chunk","created":1700000000_u64,"model":"fixture-model","choices":[{"index":0,"delta":{},"finish_reason":completion["choices"][0]["finish_reason"]}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}});
        ResponseTemplate::new(200).set_body_raw(
            format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n"),
            "text/event-stream",
        )
    }
}

#[test]
fn refused_tool_call_does_not_mark_an_earlier_turns_result_with_the_same_call_id_as_failed() {
    runtime_without_master_key().block_on(async {
        tokio::spawn(scenario()).await.unwrap();
    });
}

async fn scenario() {
    let (managed, chat) = (MockServer::start().await, MockServer::start().await);
    let fire_at = chrono::Utc::now() + chrono::Duration::seconds(30);
    Mock::given(method("POST"))
        .and(path_regex(r"/v1/chat/completions$"))
        .and(header("authorization", "Bearer operator-chat-key"))
        .respond_with(Script {
            fire_at: fire_at.to_rfc3339(),
        })
        .mount(&chat)
        .await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(500))
        .mount(&managed)
        .await;
    PointedInference::install(&managed.uri());
    openhuman_core::storage::install(Arc::new(tinystoragedrivers_testkit::MemoryBackend::new()));

    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(InMemorySessionStores::new());
    let mut saas = SaasConfig::new(root.path());
    saas.chat_provider = Some(
        DirectProfileProvider::openai_compatible(
            format!("{}/v1", chat.uri()),
            "operator-chat-key",
            "fixture-model",
        )
        .unwrap(),
    );
    saas.cron.agent_jobs_only = true;
    let runtime = ProfileRuntime::builder(saas)
        .configure(|config| config.lease_ttl_secs = 3600)
        .tool_scope(ToolScopeSpec::Named(
            ["cron", "current_time", "resolve_time", "memory"]
                .map(String::from)
                .to_vec(),
        ))
        .can_use_tool(|_| Box::pin(async { ToolHookDecision::default() }))
        .channel_permissions([("web_channel", PermissionLevel::Execute)])
        .session_store(store.clone())
        .build()
        .await
        .unwrap();
    runtime.provision("alice").await.unwrap();
    let handle = runtime.open("alice").await.unwrap();

    // Turn 1: the `cron` call (id `call_1`) succeeds. Turn 2: the same id is
    // used for a shell job the profile refuses.
    handle.chat("t1", "schedule it").await.unwrap();
    handle.chat("t1", "make a shell job").await.unwrap();
    let requests = chat_requests(&chat).await;
    assert!(
        requests
            .iter()
            .any(|r| String::from_utf8_lossy(&r.body).contains("only agent cron jobs are allowed")),
        "the refusal reaches the model"
    );

    let transcript = store
        .for_agent("alice~default")
        .transcripts
        .root_for_thread("t1")
        .expect("the thread is in the store")
        .read_session()
        .unwrap()
        .unwrap();
    let scheduled: Vec<_> = transcript
        .messages
        .iter()
        .filter(|m| m.role == "tool" && m.content.contains("\"job_type\": \"agent\""))
        .collect();
    assert_eq!(scheduled.len(), 1, "the successful cron result is recorded");
    assert!(
        scheduled[0].tool_failure.is_none(),
        "the successful cron result keeps its status"
    );
    handle.chat("t1", "and once more").await.unwrap();
}
