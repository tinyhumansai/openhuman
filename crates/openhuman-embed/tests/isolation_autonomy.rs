//! Each agent on one runtime answers policy questions from its own tier and
//! approval settings, never from the runtime's.
//!
//! The runtime boots at the widest setting — Full tier with every call
//! auto-approved — so any agent that reads the process policy instead of its
//! own shows up as a call that ran when it should have been refused or parked.

mod common;

use common::{
    chat_requests, eventually, offline_config, route, runtime, scripted_provider, stub_backend,
    tool_call_completion, tool_results,
};
use openhuman_core::security::AutonomyLevel;
use openhuman_embed::{
    Access, AgentDefinitionSpec, AgentSpec, AgentTurnOrigin, ApprovalDecision, Runtime,
    ToolScopeSpec, Workspace,
};

fn shell_writes(file: &std::path::Path) -> serde_json::Value {
    tool_call_completion(
        "shell",
        &serde_json::json!({ "command": format!("touch {}", file.display()) }).to_string(),
    )
}

fn web_origin(thread: &str) -> AgentTurnOrigin {
    AgentTurnOrigin::WebChat {
        thread_id: thread.to_string(),
        client_id: format!("client-{thread}"),
        request_id: None,
    }
}

fn shell_only() -> AgentDefinitionSpec {
    AgentDefinitionSpec::new().tools(ToolScopeSpec::Named(vec!["shell".to_string()]))
}

#[test]
fn every_agent_answers_policy_from_its_own_tier() {
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
                .build()
                .await
                .expect("runtime builds");
            let scratch = tempfile::tempdir().expect("scratch dir");

            // ── A: read-only; its shell write is refused ──
            let a_file = scratch.path().join("a-wrote");
            let a_provider = scripted_provider(vec![shell_writes(&a_file)], "a-done").await;
            let a = runtime
                .agent(
                    AgentSpec::new("readonly-a")
                        .provider(route(&a_provider, "a-model"))
                        .access(Access::readonly().trust(
                            scratch.path().display().to_string(),
                            openhuman_embed::TrustedAccess::ReadWrite,
                        ))
                        .definition(shell_only()),
                )
                .expect("a instantiates");
            assert_eq!(a.config().autonomy.level, AutonomyLevel::ReadOnly);
            let a_out = a.run("write the marker").await.expect("a's turn returns");
            assert!(!a_file.exists(), "a read-only agent must not write");
            // The scripted final reply is advisory; the tool result sent back
            // to the model proves the policy denied execution.
            let a_requests = chat_requests(&a_provider).await;
            let a_denial =
                tool_results(a_requests.get(1).expect("a continued after its tool call"));
            assert!(
                a_denial.contains("[policy-blocked]") && a_denial.contains("read-only mode"),
                "a's shell call is refused by its own tier: {}",
                a_denial
            );
            assert_eq!(
                a_out.reply, "a-done",
                "the model continues after the refused write"
            );

            // ── B: supervised, shell on its own allowlist; runs without parking ──
            let b_file = scratch.path().join("b-wrote");
            let b_provider = scripted_provider(vec![shell_writes(&b_file)], "b-done").await;
            let b = runtime
                .agent(
                    AgentSpec::new("allowlisted-b")
                        .provider(route(&b_provider, "b-model"))
                        .access(
                            Access::supervised()
                                .origin(web_origin("thread-b"))
                                .auto_approve(["shell"])
                                .auto_approve_all(false)
                                .trust(
                                    scratch.path().display().to_string(),
                                    openhuman_embed::TrustedAccess::ReadWrite,
                                ),
                        )
                        .definition(shell_only()),
                )
                .expect("b instantiates");
            tokio::time::timeout(std::time::Duration::from_secs(60), b.run("write"))
                .await
                .expect("b must not park")
                .expect("b's turn returns");
            assert!(b_file.exists(), "b's own allowlist lets shell run");
            assert!(b.approvals().pending().unwrap().is_empty());

            // ── C: supervised with no grants; parks although the runtime auto-approves ──
            let c_file = scratch.path().join("c-wrote");
            let c_provider = scripted_provider(vec![shell_writes(&c_file)], "c-done").await;
            let c = runtime
                .agent(
                    AgentSpec::new("supervised-c")
                        .provider(route(&c_provider, "c-model"))
                        .access(
                            Access::supervised()
                                .origin(web_origin("thread-c"))
                                .auto_approve(Vec::<String>::new())
                                .auto_approve_all(false)
                                .trust(
                                    scratch.path().display().to_string(),
                                    openhuman_embed::TrustedAccess::ReadWrite,
                                ),
                        )
                        .definition(shell_only()),
                )
                .expect("c instantiates");
            let c_turn = {
                let c = c.clone();
                tokio::spawn(async move { c.run("write").await })
            };
            let parked = eventually("c to park", || {
                c.approvals()
                    .pending()
                    .ok()
                    .and_then(|rows| rows.into_iter().next())
            })
            .await;
            assert_eq!(parked.tool_name, "shell");
            assert_eq!(parked.agent_id.as_deref(), Some("supervised-c"));
            assert!(!c_file.exists(), "a parked call has not run");
            assert!(b.approvals().pending().unwrap().is_empty());
            c.approvals()
                .decide(&parked.request_id, ApprovalDecision::Deny)
                .expect("c decides its own request");
            c_turn
                .await
                .unwrap()
                .expect("c's turn returns after the denial");
            assert!(!c_file.exists(), "a denied call never runs");

            // ── D: one action an hour; its second shell call is rate-limited ──
            let d_first = scratch.path().join("d-first");
            let d_second = scratch.path().join("d-second");
            let d_provider = scripted_provider(
                vec![shell_writes(&d_first), shell_writes(&d_second)],
                "d-done",
            )
            .await;
            let d = runtime
                .agent(
                    AgentSpec::new("budgeted-d")
                        .provider(route(&d_provider, "d-model"))
                        .access(Access::full().trust(
                            scratch.path().display().to_string(),
                            openhuman_embed::TrustedAccess::ReadWrite,
                        ))
                        .definition(shell_only())
                        .config(|config| config.autonomy.max_actions_per_hour = 1),
                )
                .expect("d instantiates");
            d.run("write twice").await.expect("d's turn returns");
            assert!(d_first.exists(), "d's first action is within budget");
            assert!(!d_second.exists(), "d's second action is over budget");

            let e_file = scratch.path().join("e-wrote");
            let e_provider = scripted_provider(vec![shell_writes(&e_file)], "e-done").await;
            let e = runtime
                .agent(
                    AgentSpec::new("sibling-e")
                        .provider(route(&e_provider, "e-model"))
                        .access(Access::full().trust(
                            scratch.path().display().to_string(),
                            openhuman_embed::TrustedAccess::ReadWrite,
                        ))
                        .definition(shell_only()),
                )
                .expect("e instantiates");
            e.run("write").await.expect("e's turn returns");
            assert!(e_file.exists(), "d's budget does not limit a sibling");
        })
        .await
        .expect("test task");
    });
}
