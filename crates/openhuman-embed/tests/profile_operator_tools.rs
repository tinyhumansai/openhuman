//! An operator can admit `cron`, `current_time` and `resolve_time` to profiles
//! by naming them in the tool scope, cap tool rounds per turn, and run on a
//! direct provider with no backend credential. One runtime and one test:
//! SaaS operating mode is process-wide.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{
    chat_completion, chat_requests, runtime_without_master_key, tool_names, PointedTransport,
};
use openhuman_embed::seams::ToolHookDecision;
use openhuman_embed::{
    CronDelivery, CronReply, DirectProfileProvider, JobOrigin, PermissionLevel, ProfileRuntime,
    SaasConfig, ToolScopeSpec,
};
use serde_json::{json, Value};
use wiremock::matchers::{header, method, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const CAP: usize = 3;

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
        let completion = if prompt.contains("loop forever") {
            call("current_time", json!({}))
        } else if prompt.contains("schedule it") && results == 0 {
            call(
                "cron",
                json!({"action":"add","job_type":"agent","name":"ping",
                    "schedule":{"kind":"at","at":self.fire_at},"prompt":"reminder ping"}),
            )
        } else if prompt.contains("shell job") && results == 0 {
            call(
                "cron",
                json!({"action":"add","job_type":"shell","name":"sh",
                    "schedule":{"kind":"at","at":self.fire_at},"command":"echo hi"}),
            )
        } else if prompt.contains("reminder ping") {
            chat_completion("pong")
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

#[derive(Default)]
struct Recorder(Mutex<Vec<CronReply>>);

#[async_trait::async_trait]
impl CronDelivery for Recorder {
    async fn deliver(&self, reply: CronReply) -> Result<(), String> {
        self.0.lock().unwrap().push(reply);
        Ok(())
    }
}

#[test]
fn operator_named_cron_and_time_tools_run_on_a_direct_provider_within_the_iteration_cap() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn,openhuman_core=info")
        .with_test_writer()
        .try_init();
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
    // No managed credential exists, so any managed traffic is a failure.
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(500))
        .mount(&managed)
        .await;
    PointedTransport::install(&managed.uri());
    openhuman_core::storage::install(Arc::new(openhuman_core::storage::MemoryStorage::new()));

    let root = tempfile::tempdir().unwrap();
    let recorder = Arc::new(Recorder::default());
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
    saas.max_tool_iterations = Some(CAP);
    let runtime = ProfileRuntime::builder(saas)
        .configure(|config| config.lease_ttl_secs = 3600)
        .tool_scope(ToolScopeSpec::Named(
            ["cron", "current_time", "resolve_time", "memory"]
                .map(String::from)
                .to_vec(),
        ))
        .can_use_tool(|_| Box::pin(async { ToolHookDecision::Proceed }))
        .channel_permissions([("web_channel", PermissionLevel::Execute)])
        .cron_delivery(recorder.clone())
        .build()
        .await
        .unwrap();
    // No `set_credential`: a direct provider needs no backend credential.
    runtime.provision("alice").await.unwrap();
    let handle = runtime.open("alice").await.unwrap();

    handle.chat("t0", "list the tools").await.unwrap();
    let requests = chat_requests(&chat).await;
    let mut names = tool_names(&requests[0]);
    names.sort();
    // `cron` and `memory` are deferred tools: registered, reached through the
    // harness's `tool_search` bridge instead of being listed up front.
    assert_eq!(names, ["current_time", "resolve_time", "tool_search"]);

    // An agent job made through the tool binds to this thread and fires.
    handle.chat("t1", "schedule it").await.unwrap();
    let later = chrono::Utc::now() + chrono::Duration::hours(1);
    assert_eq!(runtime.sweep_cron_at(later).await.ran, 1);
    tokio::time::timeout(Duration::from_secs(30), async {
        while recorder.0.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    {
        let delivered = recorder.0.lock().unwrap();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].text, "pong");
        assert_eq!(delivered[0].thread_id.as_deref(), Some("t1"));
        assert_eq!(
            delivered[0].origin,
            Some(JobOrigin::Web {
                thread_id: "t1".into(),
                agent_id: None
            })
        );
    }

    // A shell job through the tool is refused, and nothing is left to run.
    handle.chat("t2", "make a shell job").await.unwrap();
    let requests = chat_requests(&chat).await;
    assert!(
        requests
            .iter()
            .any(|r| String::from_utf8_lossy(&r.body).contains("only agent cron jobs are allowed")),
        "the refusal reaches the model"
    );
    assert_eq!(runtime.sweep_cron_at(later).await.ran, 0);

    // A model that never stops calling tools is cut off at the operator's cap:
    // the harness stops at CAP model calls and the turn's recovery adds a few
    // more, nowhere near the orchestrator's own cap of 200.
    let before = chat_requests(&chat).await.len();
    let _ = handle.chat("t3", "loop forever").await;
    let rounds = chat_requests(&chat).await.len() - before;
    assert!(
        (CAP..3 * CAP).contains(&rounds),
        "{rounds} model rounds for a cap of {CAP}"
    );

    let managed_paths: Vec<_> = managed
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.url.path().to_owned())
        .filter(|p| p.contains("chat") || p.contains("completions") || p.contains("openai"))
        .collect();
    assert!(
        managed_paths.is_empty(),
        "managed inference: {managed_paths:?}"
    );
}
