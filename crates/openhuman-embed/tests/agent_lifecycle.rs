//! Removing an agent from a running runtime: its turns end, its parked
//! approvals are denied, its context and MCP host are released, its id is
//! free again, and the runtime's agent cap counts only live agents.

mod common;

use std::time::{Duration, Instant};

use common::{
    eventually, offline_config, provider, route, runtime, scripted_provider, stub_backend,
    tool_call_completion,
};
use openhuman_core::core::events::DomainEvent;
use openhuman_core::core::runtime::AgentContextRegistry;
use openhuman_core::security::AutonomyLevel;
use openhuman_embed::{
    Access, Agent, AgentDefinitionSpec, AgentError, AgentSpec, AgentTurnOrigin, CoreError, Runtime,
    ToolScopeSpec, TrustedAccess, Workspace,
};

fn supervised(scratch: &std::path::Path) -> Access {
    Access::supervised()
        .origin(AgentTurnOrigin::WebChat {
            thread_id: "lifecycle-thread".to_string(),
            client_id: "lifecycle-client".to_string(),
            request_id: None,
        })
        .auto_approve(Vec::<String>::new())
        .auto_approve_all(false)
        .trust(scratch.display().to_string(), TrustedAccess::ReadWrite)
}

async fn parking_agent(
    runtime: &Runtime,
    id: &str,
    scratch: &std::path::Path,
) -> (Agent, wiremock::MockServer) {
    let file = scratch.join(format!("{id}-wrote"));
    let call = tool_call_completion(
        "shell",
        &serde_json::json!({ "command": format!("touch {}", file.display()) }).to_string(),
    );
    let provider = scripted_provider(vec![call], &format!("{id}-done")).await;
    let agent = runtime
        .agent(
            AgentSpec::new(id)
                .provider(route(&provider, &format!("{id}-model")))
                .access(supervised(scratch))
                .definition(
                    AgentDefinitionSpec::new()
                        .tools(ToolScopeSpec::Named(vec!["shell".to_string()])),
                ),
        )
        .expect("agent instantiates");
    (agent, provider)
}

async fn slow_provider(delay: Duration) -> wiremock::MockServer {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(common::chat_completion("too late"))
                .set_delay(delay),
        )
        .mount(&server)
        .await;
    server
}

fn removed(result: Result<openhuman_embed::TurnOutcome, CoreError>, id: &str) {
    match result {
        Err(CoreError::AgentRemoved { agent_id, .. }) => assert_eq!(agent_id, id),
        other => panic!("{id}'s turn should end with AgentRemoved: {other:?}"),
    }
}

#[test]
fn removing_an_agent_releases_everything_it_held() {
    let _ = env_logger::builder().is_test(true).try_init();
    runtime().block_on(async {
        tokio::spawn(async move {
            let backend = stub_backend().await;
            let mut config = offline_config();
            config.autonomy.enabled = true;
            config.autonomy.level = AutonomyLevel::Full;
            config.autonomy.auto_approve_all = true;
            let runtime = Runtime::builder()
                .config(config)
                .workspace(Workspace::Ephemeral)
                .backend_url(backend.uri())
                .access(Access::full())
                .max_agents(3)
                .build()
                .await
                .expect("runtime builds");
            let mut events = openhuman_core::core::bus::BUS
                .get()
                .expect("the runtime initialised the bus")
                .receiver();
            let scratch = tempfile::tempdir().expect("scratch dir");

            // The removal barrier must govern every shared-gate entry point,
            // including RPC/chat calls made outside the Embed approvals facade.
            // Close the exact instance scope before its denial snapshot runs.
            let (closing, _closing_provider) =
                parking_agent(&runtime, "closing", scratch.path()).await;
            let closing_turn = {
                let closing = closing.clone();
                tokio::spawn(async move { closing.turn("write the marker").send().await })
            };
            let closing_request = eventually("closing agent to park", || {
                closing.approvals().pending().ok()?.into_iter().next()
            })
            .await;
            let closing_context = AgentContextRegistry::get("closing").unwrap();
            closing_context
                .host_overrides()
                .unwrap()
                .approval_scope()
                .unwrap()
                .close("agent_removed");
            let gate = openhuman_core::security::approval::ApprovalGate::try_global().unwrap();
            for decision in [
                openhuman_embed::ApprovalDecision::ApproveOnce,
                openhuman_embed::ApprovalDecision::ApproveAlwaysForTool,
                openhuman_embed::ApprovalDecision::Deny,
            ] {
                assert!(
                    gate.decide(&closing_request.request_id, decision)
                        .unwrap()
                        .is_none(),
                    "a closed instance must refuse direct gate decisions"
                );
                assert!(gate
                    .decide_for_agent("closing", &closing_request.request_id, decision)
                    .unwrap()
                    .is_none());
            }
            assert_eq!(
                gate.classify_decide_miss(&closing_request.request_id),
                openhuman_core::security::approval::gate::DecideMiss::AlreadyResolved,
                "a removal barrier is a benign decision miss, not a lost registration"
            );
            for owner in [None, Some("closing")] {
                assert!(openhuman_core::security::approval::rpc::approval_decide(
                    &closing_request.request_id,
                    openhuman_embed::ApprovalDecision::ApproveOnce,
                    owner,
                )
                .await
                .is_err());
            }
            assert_eq!(closing.approvals().pending().unwrap().len(), 1);
            runtime.remove_agent("closing").await.unwrap();
            removed(closing_turn.await.unwrap(), "closing");
            assert!(!scratch.path().join("closing-wrote").exists());
            assert!(gate
                .list_pending_for_agent(Some("closing"))
                .unwrap()
                .is_empty());

            // Reusing the public id installs a fresh barrier: the new request
            // remains answerable through the same context-free gate entry point.
            let (replacement, _replacement_provider) =
                parking_agent(&runtime, "closing", scratch.path()).await;
            let replacement_turn = {
                let replacement = replacement.clone();
                tokio::spawn(async move { replacement.turn("write the marker").send().await })
            };
            let replacement_request = eventually("replacement to park", || {
                replacement.approvals().pending().ok()?.into_iter().next()
            })
            .await;
            assert_ne!(replacement_request.request_id, closing_request.request_id);
            assert!(gate
                .decide(
                    &replacement_request.request_id,
                    openhuman_embed::ApprovalDecision::ApproveOnce,
                )
                .unwrap()
                .is_some());
            assert!(replacement_turn.await.unwrap().is_ok());
            assert!(scratch.path().join("closing-wrote").exists());
            runtime.remove_agent("closing").await.unwrap();

            // ── an unknown id is refused ──
            let unknown = runtime
                .remove_agent("nope")
                .await
                .expect_err("nothing to remove");
            assert!(
                matches!(&unknown, AgentError::UnknownId(id) if id == "nope"),
                "{unknown:?}"
            );

            // ── a parked agent is removed ──
            let (parker, _parker_provider) =
                parking_agent(&runtime, "parker", scratch.path()).await;
            let workspace_dir = parker.workspace_dir().to_path_buf();
            let turn = {
                let parker = parker.clone();
                tokio::spawn(async move { parker.turn("write the marker").send().await })
            };
            let request = eventually("parker to park", || {
                parker
                    .approvals()
                    .pending()
                    .ok()
                    .and_then(|rows| rows.into_iter().next())
            })
            .await;
            let old_context =
                AgentContextRegistry::get("parker").expect("agent context registered");

            runtime
                .remove_agent("parker")
                .await
                .expect("parker is removed");
            removed(turn.await.expect("turn task"), "parker");
            assert!(parker.approvals().pending().unwrap().is_empty());
            // A turn admitted before removal may reach the gate after the denial
            // snapshot. Its retained context must refuse registration entirely.
            let gate = openhuman_core::security::approval::ApprovalGate::try_global().unwrap();
            let late = tokio::time::timeout(
                Duration::from_secs(10),
                openhuman_core::core::runtime::CoreContext::scope(
                    old_context,
                    openhuman_core::agent::turn_origin::with_origin(
                        AgentTurnOrigin::WebChat {
                            thread_id: "late-lifecycle-thread".into(),
                            client_id: "late-lifecycle-client".into(),
                            request_id: None,
                        },
                        gate.intercept_forced(
                            "shell",
                            "late removal registration",
                            serde_json::json!({}),
                        ),
                    ),
                ),
            )
            .await
            .expect("a removed instance cannot park a late approval");
            assert!(matches!(
                late,
                openhuman_core::security::approval::GateOutcome::Deny { reason }
                    if reason.contains("agent_removed")
            ));
            assert!(parker.approvals().pending().unwrap().is_empty());
            let decided = loop {
                match tokio::time::timeout(Duration::from_secs(10), events.recv())
                    .await
                    .expect("ApprovalDecided arrives")
                {
                    Some(DomainEvent::ApprovalDecided {
                        request_id,
                        decision,
                        resolution,
                        agent_id,
                        ..
                    }) if request_id == request.request_id => {
                        break (decision, resolution, agent_id)
                    }
                    Some(_) => {}
                    None => panic!("bus closed"),
                }
            };
            assert_eq!(decided.0, "deny");
            assert_eq!(decided.1.as_deref(), Some("agent_removed"));
            assert_eq!(decided.2.as_deref(), Some("parker"));
            assert!(
                AgentContextRegistry::get("parker").is_none(),
                "the context left the registry, so its cron jobs stay dormant"
            );
            assert!(
                openhuman_core::mcp::host::take_agent_host(&workspace_dir, "parker").is_none(),
                "no MCP host is kept for a removed agent"
            );
            removed(parker.turn("again").send().await, "parker");
            assert!(
                !scratch.path().join("parker-wrote").exists(),
                "the denied call never ran"
            );

            // ── its id is free again, on a handle that works ──
            let reborn_provider = provider("reborn").await;
            let reborn = runtime
                .agent(AgentSpec::new("parker").provider(route(&reborn_provider, "reborn-model")))
                .expect("the id is reusable");
            let outcome = reborn.run("hello").await.expect("the new agent runs");
            assert!(outcome.reply.contains("reborn"), "{}", outcome.reply);
            drop(reborn);
            drop(parker);

            // ── a turn in flight ends when its agent is removed ──
            let slow = slow_provider(Duration::from_secs(60)).await;
            let sleeper = runtime
                .agent(AgentSpec::new("sleeper").provider(route(&slow, "slow-model")))
                .expect("sleeper instantiates");
            let turn = {
                let sleeper = sleeper.clone();
                tokio::spawn(async move { sleeper.turn("take your time").send().await })
            };
            let mut waited = 0;
            while common::chat_requests(&slow).await.is_empty() {
                assert!(waited < 1000, "the slow provider never saw the turn");
                tokio::time::sleep(Duration::from_millis(10)).await;
                waited += 1;
            }
            let started = Instant::now();
            runtime
                .remove_agent("sleeper")
                .await
                .expect("sleeper is removed");
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "removal does not wait for the provider: {:?}",
                started.elapsed()
            );
            removed(turn.await.expect("turn task"), "sleeper");
            drop(sleeper);

            // ── purge deletes the agent's home ──
            let purged_provider = provider("purged").await;
            let purged = runtime
                .agent(AgentSpec::new("purged").provider(route(&purged_provider, "p-model")))
                .expect("purged instantiates");
            purged.run("leave a transcript").await.expect("turn");
            let home = purged.home_dir().to_path_buf();
            assert!(home.exists());
            runtime
                .remove_agent("purged")
                .purge()
                .await
                .expect("purged is removed");
            assert!(!home.exists(), "purge deleted {}", home.display());
            drop(purged);

            // ── the cap counts live agents only ──
            let cap_provider = provider("capped").await;
            let spec = |id: &str| AgentSpec::new(id).provider(route(&cap_provider, "cap-model"));
            let first = runtime.agent(spec("cap-1")).expect("1st");
            let _second = runtime.agent(spec("cap-2")).expect("2nd");
            let _third = runtime.agent(spec("cap-3")).expect("3rd");
            let refused = runtime.agent(spec("cap-4")).expect_err("over the cap");
            assert!(
                matches!(refused, AgentError::AgentLimit { limit: 3 }),
                "{refused:?}"
            );
            runtime
                .remove_agent("cap-1")
                .await
                .expect("cap-1 is removed");
            drop(first);
            runtime.agent(spec("cap-4")).expect("room after a removal");
        })
        .await
        .expect("test task");
    });
}
