//! Profile-owned host tools and a shared memory engine stay within each tenant.
//! One runtime and one test: SaaS operating mode is process-wide.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{
    chat_completion, chat_requests, runtime_without_master_key, tool_call_completion, tool_names,
    PointedTransport,
};
use openhuman_embed::memory::api::conformance::ReferenceEngine;
use openhuman_embed::memory::api::{LearningKind, MemoryEngine, MemoryMeta, StoreItem};
use openhuman_embed::profiles::ProfileCredentialKind;
use openhuman_embed::seams::ToolHookDecision;
use openhuman_embed::{
    HostTurnTools, PermissionLevel, ProfileRuntime, RelayMessage, SaasConfig, Tool, ToolResult,
    ToolScopeSpec,
};
use serde_json::{json, Value};
use wiremock::{Mock, Request, Respond, ResponseTemplate};

struct TenantTool {
    name: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
    relay_origins: Arc<Mutex<Vec<(String, String)>>>,
}

#[async_trait::async_trait]
impl Tool for TenantTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "Report the current profile"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{}})
    }
    async fn execute_with_context(
        &self,
        args: Value,
        _: tinytools::ToolCallOptions,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let thread = context
            .and_then(tinytools::ToolRunContext::thread_id)
            .expect("every hosted tool has its durable conversation key");
        let expected = match openhuman_embed::current_turn_origin().unwrap() {
            openhuman_embed::AgentTurnOrigin::WebChat { thread_id, .. } => thread_id,
            openhuman_embed::AgentTurnOrigin::ExternalChannel { .. } => {
                "channel:telegram/sender/chat".to_owned()
            }
            other => panic!("unexpected origin {other:?}"),
        };
        assert_eq!(thread, expected);
        self.execute(args).await
    }
    async fn execute(&self, _: Value) -> anyhow::Result<ToolResult> {
        let tenant = openhuman_embed::current_tenant()?;
        let profile = tenant.profile.expect("profile tool always has a tenant");
        match openhuman_embed::current_turn_origin().expect("a host turn has its original route") {
            openhuman_embed::AgentTurnOrigin::WebChat { thread_id, .. } => {
                assert!(matches!(thread_id.as_str(), "t1" | "callback-denied"))
            }
            openhuman_embed::AgentTurnOrigin::ExternalChannel {
                channel,
                sender,
                reply_target,
                message_id,
                ..
            } => {
                assert_eq!(channel, "telegram");
                assert_eq!(sender.as_deref(), Some("sender"));
                assert_eq!(reply_target, "chat");
                self.relay_origins
                    .lock()
                    .unwrap()
                    .push((profile.clone(), message_id));
            }
            origin => panic!("unexpected turn origin: {origin:?}"),
        }
        self.calls
            .lock()
            .unwrap()
            .push(format!("{}:{profile}", self.name));
        Ok(ToolResult::success(profile))
    }
}

struct CallTools;
impl Respond for CallTools {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let messages = body["messages"].as_array().unwrap();
        let last_user = messages
            .iter()
            .rposition(|message| message["role"] == "user")
            .unwrap();
        let has_tool_results = messages[last_user + 1..]
            .iter()
            .any(|message| message["role"] == "tool");
        let completion = if common::last_user_message(&body).contains("deny callback request")
            && !has_tool_results
        {
            tool_call_completion("tenant_probe", r#"{"deny":true}"#)
        } else if common::last_user_message(&body).contains("unsafe fixture request") {
            let mut completion = tool_call_completion("shell", "{}");
            completion["choices"][0]["message"]["tool_calls"] = json!(["shell", "delegate", "http_request", "run_workflow"].iter().enumerate().map(|(i, name)| json!({"id":format!("unsafe_{i}"),"type":"function","function":{"name":name,"arguments":"{}"}})).collect::<Vec<_>>());
            completion
        } else if has_tool_results
            || !tool_names(request)
                .iter()
                .any(|name| name == "tenant_probe")
        {
            let message = common::last_user_message(&body);
            let user = if message.contains("alice") {
                "alice"
            } else {
                "bob"
            };
            let channel = if message.contains("\"telegram\" channel") {
                "relay"
            } else {
                "chat"
            };
            let reopened = if message.contains("after reopening") {
                "reopened-"
            } else {
                ""
            };
            chat_completion(&format!("done: {reopened}{channel}:{user}"))
        } else {
            let mut completion = tool_call_completion("tenant_probe", "{}");
            let names = ["tenant_probe"];
            completion["choices"][0]["message"]["tool_calls"] = json!(names.iter().enumerate().map(|(i, name)| json!({"id":format!("call_{i}"),"type":"function","function":{"name":name,"arguments":"{}"}})).collect::<Vec<_>>());
            let message = common::last_user_message(&body);
            let user = if message.contains("alice") {
                "alice"
            } else {
                "bob"
            };
            completion["choices"][0]["message"]["tool_calls"].as_array_mut().unwrap().push(json!({
                "id":"native-memory", "type":"function", "function":{"name":"memory", "arguments":json!({"action":"learn", "text":format!("{user}-native-learning")}).to_string()}
            }));
            completion
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
        let chunk = json!({"id":"fixture", "object":"chat.completion.chunk", "created":1700000000_u64, "model":"fixture", "choices":[{"index":0,"delta":delta,"finish_reason":null}]});
        let done = json!({"id":"fixture", "object":"chat.completion.chunk", "created":1700000000_u64, "model":"fixture", "choices":[{"index":0,"delta":{},"finish_reason":completion["choices"][0]["finish_reason"]}], "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}});
        ResponseTemplate::new(200).set_body_raw(
            format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n"),
            "text/event-stream",
        )
    }
}

#[test]
fn chat_and_relay_use_only_selected_tools_inside_their_profile() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn")
        .with_test_writer()
        .try_init();
    runtime_without_master_key().block_on(async {
        tokio::spawn(scenario()).await.unwrap();
    });
}

async fn scenario() {
    let inference = wiremock::MockServer::start().await;
    Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path_regex(r"chat/completions$"))
        .respond_with(CallTools)
        .with_priority(1)
        .mount(&inference)
        .await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success":true,"data":[]})))
        .with_priority(10)
        .mount(&inference)
        .await;
    PointedTransport::install(&inference.uri());
    let root = tempfile::tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let factory_calls = calls.clone();
    let relay_origins = Arc::new(Mutex::new(Vec::new()));
    let factory_relay_origins = relay_origins.clone();
    let sessions = Arc::new(Mutex::new(Vec::new()));
    let factory_sessions = sessions.clone();
    let engine = Arc::new(ReferenceEngine::new());
    let runtime = ProfileRuntime::builder(SaasConfig::new(root.path()))
        // Keep automatic renewals beyond this bounded scenario so the final
        // synthetic expiry check cannot race a heartbeat.
        .configure(|config| config.lease_ttl_secs = 3600)
        .tools(move |turn| {
            let profile = openhuman_embed::current_tenant().unwrap().profile.unwrap();
            factory_sessions
                .lock()
                .unwrap()
                .push((profile, turn.session_id().map(str::to_owned)));
            HostTurnTools::advertised(
                [
                    "tenant_probe",
                    "hidden",
                    "shell",
                    "delegate",
                    "curl",
                    "install_workflow_from_url",
                    "http_request",
                    "run_workflow",
                    "delegate_to",
                ]
                .into_iter()
                .map(|name| {
                    Box::new(TenantTool {
                        name,
                        calls: factory_calls.clone(),
                        relay_origins: factory_relay_origins.clone(),
                    }) as Box<dyn Tool>
                })
                .collect(),
            )
        })
        .tool_scope(ToolScopeSpec::Named(vec![
            "tenant_probe".into(),
            "memory".into(),
            "shell".into(),
            "delegate".into(),
            "curl".into(),
            "install_workflow_from_url".into(),
            "http_request".into(),
            "run_workflow".into(),
            "delegate_to".into(),
        ]))
        .can_use_tool(|context| {
            Box::pin(async move {
                if context.arguments.get("deny").and_then(Value::as_bool) == Some(true) {
                    ToolHookDecision::Deny("operator refused this call".into())
                } else {
                    ToolHookDecision::Proceed
                }
            })
        })
        .channel_permissions([
            ("cli", PermissionLevel::Execute),
            ("web_channel", PermissionLevel::Execute),
            ("telegram", PermissionLevel::Execute),
        ])
        .memory_engine(engine.clone())
        .build()
        .await
        .unwrap();
    let mut handles = Vec::new();
    for user in ["alice", "bob"] {
        let provisioned = runtime.provision(user).await.unwrap();
        runtime
            .set_credential(
                &provisioned.profile_id,
                ProfileCredentialKind::ApiKey,
                "fixture-key",
            )
            .await
            .unwrap();
        let handle = runtime.open(user).await.unwrap();
        assert!(handle.context().embedder_config().unwrap().autonomy.enabled);
        assert_eq!(
            handle
                .context()
                .embedder_config()
                .unwrap()
                .memory
                .root
                .as_deref(),
            Some(format!("user:{user}").as_str())
        );
        let profile = handle
            .scope(async {
                openhuman_core::profiles::host::current(
                    &openhuman_core::core::runtime::DispatchCtx::ambient(),
                )
                .unwrap()
            })
            .await;
        profile.lease_fence().renewed(0);
        assert!(
            !handle
                .scope(async {
                    openhuman_core::memory::engine::is_on(
                        handle.context().embedder_config().unwrap(),
                    )
                })
                .await,
            "expired profile memory must fail closed"
        );
        profile.lease_fence().renewed(u64::MAX);
        tokio::time::timeout(
            Duration::from_secs(30),
            handle.chat(
                "t1",
                &format!("remember {user}-native-learning and call tenant_probe"),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        handles.push(handle);
    }
    for handle in &handles {
        let mut events = handle.events();
        let accepted = handle
            .relay_inbound(RelayMessage::new(
                "telegram",
                "chat",
                "sender",
                "message",
                format!("{}: call tenant_probe", handle.id()),
            ))
            .await
            .unwrap();
        assert!(accepted.accepted);
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let event = events.recv().await.unwrap();
                if event.event == "channel_outbound" {
                    break;
                }
            }
        })
        .await
        .unwrap();
    }
    assert_eq!(
        *calls.lock().unwrap(),
        [
            "tenant_probe:alice",
            "tenant_probe:bob",
            "tenant_probe:alice",
            "tenant_probe:bob"
        ]
    );
    assert_eq!(
        *sessions.lock().unwrap(),
        [
            ("alice".into(), Some("t1".into())),
            ("bob".into(), Some("t1".into())),
            ("alice".into(), Some("channel:telegram/sender/chat".into())),
            ("bob".into(), Some("channel:telegram/sender/chat".into())),
        ]
    );
    let executions_before_denied_turn = calls.lock().unwrap().len();
    assert!(
        handles[0]
            .chat("denied", "unsafe fixture request")
            .await
            .is_err(),
        "unregistered unsafe model calls must fail closed"
    );
    assert_eq!(calls.lock().unwrap().len(), executions_before_denied_turn);
    assert!(
        handles[0]
            .chat("callback-denied", "deny callback request")
            .await
            .is_err(),
        "the operator callback must refuse even a registered safe name"
    );
    assert_eq!(calls.lock().unwrap().len(), executions_before_denied_turn);
    // Persistent hooks fail closed if reused under a different profile or
    // after the owning lease is fenced, before the host callback is consulted.
    let hook_context = serde_json::from_value(json!({"event":"PreToolUse", "call_id":"fixture", "tool_name":"tenant_probe", "arguments":{}, "success":null, "duration_ms":null})).unwrap();
    let alice_hook = handles[0]
        .context()
        .host_overrides()
        .unwrap()
        .tool_hooks()
        .into_iter()
        .find(|hook| hook.name() == "profile.can_use_tool")
        .unwrap();
    assert!(matches!(
        handles[0]
            .scope(alice_hook.before_tool_decision(&hook_context))
            .await,
        ToolHookDecision::Proceed
    ));
    assert!(handles[1]
        .scope(alice_hook.before_tool_decision(&hook_context))
        .await
        .is_deny());
    for name in [
        "hidden",
        "shell",
        "delegate",
        "curl",
        "http_request",
        "run_workflow",
        "delegate_to",
        "install_workflow_from_url",
    ] {
        let mut denied = hook_context.clone();
        denied.tool_name = name.to_owned();
        assert!(
            handles[0]
                .scope(alice_hook.before_tool_decision(&denied))
                .await
                .is_deny(),
            "{name} must be refused even with a proceeding host callback"
        );
    }
    let no_callback = Arc::new(
        openhuman_core::profiles::capabilities::ProfileCapabilities::new(
            None,
            Some(["tenant_probe".to_string()].into_iter().collect()),
            None,
            Default::default(),
            Vec::new(),
            None,
        ),
    );
    let alice_profile = handles[0]
        .scope(async {
            openhuman_core::profiles::current(
                    &openhuman_core::core::runtime::DispatchCtx::ambient(),
                )
                .unwrap()
        })
        .await;
    let closed_hook = no_callback.hook(handles[0].id(), alice_profile.lease_fence().clone());
    assert!(handles[0]
        .scope(closed_hook.before_tool_decision(&hook_context))
        .await
        .is_deny());
    // Closing really releases the old lease; reopening must install the same
    // immutable belt and engine on a fresh profile context, without pinning it.
    drop(alice_profile);
    let old_alice = handles.remove(0);
    let alice_id = old_alice.id().clone();
    drop(old_alice);
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match runtime.release(&alice_id).await {
                Ok(released) => {
                    assert!(released);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
    })
    .await
    .expect("completed turns leave the profile releasable");
    let reopened = runtime.open("alice").await.unwrap();
    assert!(reopened
        .threads()
        .await
        .unwrap()
        .iter()
        .any(|thread| thread.id == "t1"));
    reopened
        .chat("t1", "alice: call tenant_probe after reopening")
        .await
        .unwrap();
    let mut events = reopened.events();
    assert!(
        reopened
            .relay_inbound(RelayMessage::new(
                "telegram",
                "chat",
                "sender",
                "reopened-message",
                "alice: call tenant_probe after reopening"
            ))
            .await
            .unwrap()
            .accepted
    );
    tokio::time::timeout(Duration::from_secs(30), async {
        while events.recv().await.unwrap().event != "channel_outbound" {}
    })
    .await
    .unwrap();
    assert_eq!(
        &calls.lock().unwrap()[4..],
        ["tenant_probe:alice", "tenant_probe:alice"]
    );
    assert_eq!(
        *relay_origins.lock().unwrap(),
        [
            ("alice".into(), "message".into()),
            ("bob".into(), "message".into()),
            ("alice".into(), "reopened-message".into())
        ]
    );
    {
        let reopened_sessions = sessions.lock().unwrap();
        assert_eq!(
            &reopened_sessions[reopened_sessions.len() - 2..],
            [
                ("alice".into(), Some("t1".into())),
                ("alice".into(), Some("channel:telegram/sender/chat".into()))
            ]
        );
    }
    handles.insert(0, reopened);
    let alice_profile = handles[0]
        .scope(async {
            openhuman_core::profiles::current(
                    &openhuman_core::core::runtime::DispatchCtx::ambient(),
                )
                .unwrap()
        })
        .await;
    let alice_hook = handles[0]
        .context()
        .host_overrides()
        .unwrap()
        .tool_hooks()
        .into_iter()
        .find(|hook| hook.name() == "profile.can_use_tool")
        .unwrap();
    assert!(matches!(
        handles[0]
            .scope(alice_hook.before_tool_decision(&hook_context))
            .await,
        ToolHookDecision::Proceed
    ));
    let requests = chat_requests(&inference).await;
    assert!(requests
        .iter()
        .any(|request| !tool_names(request).is_empty()));
    for request in requests {
        let mut names = tool_names(&request);
        names.sort();
        if !names.is_empty() {
            assert_eq!(
                names,
                ["memory", "tenant_probe"],
                "the model sees the selected belt exactly"
            );
        }
    }
    assert!(
        inference
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| !request.url.path().contains("memory/experience")),
        "deferred profile memory must keep its bound host engine"
    );
    // Shared backend contents are independent of the actor: actual contract
    // requests, including a malicious global-root filter, must stay confined.
    for user in ["alice", "bob"] {
        engine
            .store(StoreItem::learning(
                format!("{user}-secret"),
                LearningKind::Fact,
                0.9,
                MemoryMeta {
                    namespace: format!("user:{user}").parse().unwrap(),
                    ..Default::default()
                },
            ))
            .await
            .unwrap();
    }
    for handle in &handles {
        assert!(
            handle
                .call::<_, Value>(
                    "openhuman.memory_items_list",
                    json!({"filter":{"reach":{"at":"","descendants":true,"inherit":true}}})
                )
                .await
                .is_err(),
            "a global reach must be refused"
        );
        let mut last_response = Value::Null;
        let response: Value = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let response: Value = handle
                    .call("openhuman.memory_items_list", json!({"limit":100}))
                    .await
                    .unwrap();
                last_response = response.clone();
                if response
                    .to_string()
                    .contains(&format!("{}-native-learning", handle.id()))
                    && response
                        .to_string()
                        .contains(&format!("done: chat:{}", handle.id()))
                    && response
                        .to_string()
                        .contains(&format!("done: relay:{}", handle.id()))
                    && (handle.id().as_str() != "alice"
                        || (response.to_string().contains("done: reopened-chat:alice")
                            && response.to_string().contains("done: reopened-relay:alice")))
                {
                    break response;
                }
                // Native writes acknowledge their durable local queue. Await the
                // queued worker's store before checking the real engine contents.
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|error| {
            panic!("memory persistence timed out: {error}; last response: {last_response}")
        });
        let text = response.to_string();
        assert!(
            text.contains(&format!("{}-secret", handle.id())),
            "{response}"
        );
        assert!(
            text.contains(&format!("done: chat:{}", handle.id())),
            "the deferred chat commit reaches the profile engine: {response}"
        );
        if handle.id().as_str() == "alice" {
            assert!(
                text.contains("done: reopened-chat:alice"),
                "reopened chat commit: {response}"
            );
            assert!(
                text.contains("done: reopened-relay:alice"),
                "reopened relay commit: {response}"
            );
        }
        assert!(
            text.contains(&format!("{}-native-learning", handle.id())),
            "native memory mutation was persisted: {response}"
        );
        let other = if handle.id().as_str() == "alice" {
            "bob"
        } else {
            "alice"
        };
        assert!(!text.contains(&format!("{other}-secret")), "{response}");
    }
    let forced_config = alice_profile.config.clone();
    let mut escaped_config = forced_config.clone();
    escaped_config.memory.root = Some("user:bob".into());
    assert!(
        !handles[0]
            .scope(async { openhuman_core::memory::engine::is_on(&escaped_config) })
            .await
    );
    alice_profile.lease_fence().renewed(0);
    assert!(
        !handles[0]
            .scope(async { openhuman_core::memory::engine::is_on(&forced_config) })
            .await,
        "an expired profile must not resolve the shared engine"
    );
    alice_profile.lease_fence().renewed(u64::MAX);
    alice_profile.lease_fence().fence();
    assert!(
        !handles[0]
            .scope(async { openhuman_core::memory::engine::is_on(&forced_config) })
            .await,
        "a latched profile must not resolve the shared engine"
    );
    assert!(handles[0]
        .scope(alice_hook.before_tool_decision(&hook_context))
        .await
        .is_deny());
    runtime.shutdown().await;
}
