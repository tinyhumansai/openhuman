//! Sub-agents belong to the agent that declared them: only it gets the
//! delegation tool, the delegated work runs on that agent's provider, and the
//! built-in catalogue ids cannot be taken as agent ids.

mod common;

use common::{
    chat_requests, offline_config, route, runtime, scripted_provider, stub_backend,
    tool_call_completion, tool_names,
};
use openhuman_embed::{
    Access, AgentDefinitionSpec, AgentError, AgentSpec, Runtime, ToolScopeSpec, Workspace,
};

const HELPER_MARK: &str = "HELPER-SUBAGENT-MARK";

#[test]
fn sub_agents_stay_with_the_agent_that_declared_them() {
    let _ = env_logger::builder().is_test(true).try_init();
    runtime().block_on(async {
        tokio::spawn(async move {
            let backend = stub_backend().await;
            let runtime_provider = scripted_provider(Vec::new(), "runtime-provider").await;
            let runtime = Runtime::builder()
                .config(offline_config())
                .workspace(Workspace::Ephemeral)
                .backend_url(backend.uri())
                .provider(route(&runtime_provider, "runtime-model"))
                .access(Access::full())
                .build()
                .await
                .expect("runtime builds");

            // ── reserved ids ──
            for reserved in ["orchestrator", "vision_agent", "summarizer"] {
                let refused = runtime.agent(AgentSpec::new(reserved));
                assert!(
                    matches!(refused, Err(AgentError::ReservedId(ref id)) if id == reserved),
                    "{reserved} must be reserved"
                );
            }
            let refused = runtime.agent(
                AgentSpec::new("lead-x").subagents([("vision_agent", AgentDefinitionSpec::new())]),
            );
            assert!(matches!(refused, Err(AgentError::ReservedId(_))));

            // ── A declares a helper; B does not ──
            let a_provider = scripted_provider(
                vec![tool_call_completion(
                    "spawn_subagent",
                    &serde_json::json!({
                        "agent_id": "a_helper",
                        "prompt": "summarise the plan",
                        "task_key": "a-helper-summary"
                    })
                    .to_string(),
                )],
                "a-done",
            )
            .await;
            let b_provider = scripted_provider(Vec::new(), "b-done").await;
            let a = runtime
                .agent(
                    AgentSpec::new("lead-a")
                        .provider(route(&a_provider, "a-model"))
                        .access(Access::full())
                        .definition(
                            AgentDefinitionSpec::new().tools(ToolScopeSpec::Named(Vec::new())),
                        )
                        .subagents([(
                            "a_helper",
                            AgentDefinitionSpec::new()
                                .system_prompt(format!("You are {HELPER_MARK}."))
                                .when_to_use("Summarises plans for lead-a.")
                                .tools(ToolScopeSpec::Named(Vec::new())),
                        )]),
                )
                .expect("a instantiates");
            let b = runtime
                .agent(
                    AgentSpec::new("plain-b")
                        .provider(route(&b_provider, "b-model"))
                        .access(Access::full())
                        .definition(
                            AgentDefinitionSpec::new().tools(ToolScopeSpec::Named(Vec::new())),
                        ),
                )
                .expect("b instantiates");

            let a_out = a.run("plan something").await.expect("a's turn returns");
            assert!(a_out.reply.contains("a-done"), "{}", a_out.reply);
            b.run("plan something").await.expect("b's turn returns");

            let a_requests = chat_requests(&a_provider).await;
            assert!(
                tool_names(&a_requests[0])
                    .iter()
                    .any(|t| t == "spawn_subagent"),
                "a is offered its own helper: {:?}",
                tool_names(&a_requests[0])
            );
            let a_body: serde_json::Value =
                serde_json::from_slice(&a_requests[0].body).expect("A's provider request JSON");
            let spawn_spec = a_body["tools"]
                .as_array()
                .expect("provider request tools")
                .iter()
                .find(|tool| {
                    tool.pointer("/function/name").and_then(|v| v.as_str())
                        == Some("spawn_subagent")
                })
                .expect("canonical spawn tool schema");
            assert_eq!(
                a_body["tools"]
                    .as_array()
                    .expect("provider request tools")
                    .iter()
                    .filter(|tool| {
                        tool.pointer("/function/name").and_then(|v| v.as_str())
                            == Some("spawn_subagent")
                    })
                    .count(),
                1,
                "A receives one canonical spawn tool"
            );
            assert_eq!(
                spawn_spec.pointer("/function/parameters/properties/agent_id/enum"),
                Some(&serde_json::json!(["a_helper"])),
                "A's spawn schema lists only its declared helper"
            );
            assert!(
                !tool_names(&a_requests[0])
                    .iter()
                    .any(|tool| tool == "delegate_a_helper"),
                "a uses the shared spawn tool instead of a generated delegate alias"
            );
            assert!(
                a_requests
                    .iter()
                    .filter(|r| !String::from_utf8_lossy(&r.body).contains(HELPER_MARK))
                    .any(|r| common::tool_results(r).contains("a_helper")),
                "the delegation was accepted for a's helper"
            );
            let mut helper_ran_on_a = false;
            for _ in 0..1000 {
                helper_ran_on_a = chat_requests(&a_provider)
                    .await
                    .iter()
                    .any(|r| String::from_utf8_lossy(&r.body).contains(HELPER_MARK));
                if helper_ran_on_a {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            assert!(
                helper_ran_on_a,
                "the detached helper turn runs on a's provider, under a's context"
            );

            let b_requests = chat_requests(&b_provider).await;
            assert!(!b_requests.is_empty());
            assert!(
                !tool_names(&b_requests[0])
                    .iter()
                    .any(|tool| tool == "spawn_subagent"),
                "an agent with no declared helpers does not receive the spawn tool"
            );
            for request in &b_requests {
                assert!(
                    !tool_names(request).iter().any(|t| t == "delegate_a_helper"),
                    "b never sees a's helper"
                );
                assert!(!String::from_utf8_lossy(&request.body).contains(HELPER_MARK));
            }
            host_only_and_disallowed_scopes_do_not_opt_into_declared_helpers(&runtime).await;
            assert_eq!(
                chat_requests(&runtime_provider).await.len(),
                0,
                "no delegated work falls back to the runtime's provider"
            );
        })
        .await
        .expect("test task");
    });
}

async fn host_only_and_disallowed_scopes_do_not_opt_into_declared_helpers(runtime: &Runtime) {
    let host_only_provider = scripted_provider(
        vec![
            tool_call_completion(
                "spawn_subagent",
                &serde_json::json!({
                    "agent_id": "host_helper",
                    "prompt": "do host-only work",
                    "task_key": "host-helper-task"
                })
                .to_string(),
            ),
            common::chat_completion("host-only done"),
        ],
        "host-only done",
    )
    .await;
    let host_only = runtime
        .agent(
            AgentSpec::new("host-only-lead")
                .provider(route(&host_only_provider, "host-only-model"))
                .definition(
                    AgentDefinitionSpec::new()
                        .bare_prompt("You are a host-managed agent.")
                        .tools(ToolScopeSpec::HostOnly),
                )
                .subagents([(
                    "host_helper",
                    AgentDefinitionSpec::new()
                        .system_prompt("HOST-ONLY-HELPER-MARK")
                        .when_to_use("host-only helper"),
                )]),
        )
        .expect("host-only agent with declared helper builds");
    host_only
        .run("try the helper")
        .await
        .expect("host-only turn returns");
    let host_requests = chat_requests(&host_only_provider).await;
    assert!(!host_requests.is_empty());
    assert!(!tool_names(&host_requests[0])
        .iter()
        .any(|name| name == "spawn_subagent"));
    assert!(host_requests
        .iter()
        .all(|request| !String::from_utf8_lossy(&request.body).contains("HOST-ONLY-HELPER-MARK")));
    let host_tool_results = host_requests
        .iter()
        .map(common::tool_results)
        .collect::<Vec<_>>()
        .join("\n");
    assert_refused_without_async_reference(&host_tool_results);

    let denied_provider = scripted_provider(
        vec![
            tool_call_completion(
                "spawn_subagent",
                &serde_json::json!({
                    "agent_id": "denied_helper",
                    "prompt": "do denied work",
                    "task_key": "denied-helper-task"
                })
                .to_string(),
            ),
            common::chat_completion("denied turn done"),
        ],
        "denied turn done",
    )
    .await;
    let denied = runtime
        .agent(
            AgentSpec::new("denied-lead")
                .provider(route(&denied_provider, "denied-model"))
                .definition(
                    AgentDefinitionSpec::new()
                        .tools(ToolScopeSpec::Named(Vec::new()))
                        .disallow_tools(["spawn_*"]),
                )
                .subagents([(
                    "denied_helper",
                    AgentDefinitionSpec::new()
                        .system_prompt("DISALLOWED-HELPER-MARK")
                        .when_to_use("disallowed helper"),
                )]),
        )
        .expect("agent with explicit spawn denial builds");
    denied
        .run("try the helper")
        .await
        .expect("disallowed turn returns");
    let denied_requests = chat_requests(&denied_provider).await;
    assert!(!denied_requests.is_empty());
    assert!(!tool_names(&denied_requests[0])
        .iter()
        .any(|name| name == "spawn_subagent"));
    assert!(denied_requests
        .iter()
        .all(|request| !String::from_utf8_lossy(&request.body).contains("DISALLOWED-HELPER-MARK")));
    let denied_tool_results = denied_requests
        .iter()
        .map(common::tool_results)
        .collect::<Vec<_>>()
        .join("\n");
    assert_refused_without_async_reference(&denied_tool_results);
}

fn assert_refused_without_async_reference(result: &str) {
    let lowercase = result.to_ascii_lowercase();
    assert!(
        lowercase.contains("not available")
            || lowercase.contains("not allowed")
            || lowercase.contains("not in the current session")
            || lowercase
                .trim_start()
                .starts_with("unknown tool `spawn_subagent`"),
        "the attempted spawn must return an explicit refusal: {result}"
    );
    for reference_field in ["task_id", "subagent_session_id", "status"] {
        assert!(
            !result.contains(reference_field),
            "a refused spawn must not return an asynchronous task reference: {result}"
        );
    }
}
