//! Flow approval surfaces use the embedded instance's workspace and removal barrier.

mod common;

use std::time::Duration;

use openhuman_core::agent::turn_origin::{self, TrustedAutomationSource};
use openhuman_core::core::events::DomainEvent;
use openhuman_core::core::runtime::{AgentContextRegistry, CoreContext};
use openhuman_core::security::approval::gate::{FlowRunContext, APPROVAL_FLOW_RUN_CONTEXT};
use openhuman_core::security::approval::{ApprovalGate, ApprovalSourceContext, GateOutcome};
use openhuman_embed::{
    Access, AgentDefinitionSpec, AgentSpec, AgentTurnOrigin, ApprovalDecision, Runtime,
    ToolScopeSpec, TrustedAccess, Workspace,
};

fn origin(flow_id: &str) -> AgentTurnOrigin {
    AgentTurnOrigin::TrustedAutomation {
        job_id: flow_id.to_owned(),
        source: TrustedAutomationSource::Workflow {
            require_approval: true,
        },
    }
}

fn run_context(flow_id: &str, run_id: &str) -> FlowRunContext {
    FlowRunContext {
        flow_id: flow_id.to_owned(),
        run_id: run_id.to_owned(),
    }
}

#[test]
fn flow_surfaces_correlate_decisions_and_closed_instances_never_publish() {
    common::runtime().block_on(async {
        tokio::spawn(async {
            let backend = common::stub_backend().await;
            let scratch = tempfile::tempdir().unwrap();
            let marker = scratch.path().join("denied-flow-tool");
            let provider = common::scripted_provider(
                vec![common::tool_call_completion(
                    "shell",
                    &serde_json::json!({"command": format!("touch '{}'", marker.display())})
                        .to_string(),
                )],
                "flow-denied",
            )
            .await;
            let runtime = Runtime::builder()
                .config(common::offline_config())
                .workspace(Workspace::Ephemeral)
                .backend_url(backend.uri())
                .access(Access::supervised().auto_approve_all(false))
                .build()
                .await
                .unwrap();
            let spec = |id: &str, flow_id: &str| {
                AgentSpec::new(id)
                    .provider(common::route(&provider, "flow-fixture"))
                    .action_dir(scratch.path())
                    .access(
                        Access::supervised()
                            .origin(origin(flow_id))
                            .auto_approve_all(false)
                            .auto_approve(Vec::<String>::new())
                            .trust(
                                scratch.path().display().to_string(),
                                TrustedAccess::ReadWrite,
                            ),
                    )
                    .definition(
                        AgentDefinitionSpec::new()
                            .tools(ToolScopeSpec::Named(vec!["shell".to_owned()])),
                    )
            };
            let agent = runtime.agent(spec("flow-agent", "flow-live")).unwrap();
            let context = AgentContextRegistry::get("flow-agent").unwrap();
            let scope = context.host_overrides().unwrap().approval_scope().unwrap();
            assert!(!scope.is_closed());
            let (workspace, _) = CoreContext::scope(
                context.clone(),
                openhuman_core::config::active_workspace_snapshot(),
            )
            .await
            .unwrap();
            let workspace_handle = openhuman_core::config::workspace_handle(&workspace);
            let gate = ApprovalGate::try_global().unwrap();
            let mut events = openhuman_core::core::bus::BUS.get().unwrap().receiver();
            let mut notifications =
                openhuman_core::desktop::notifications::bus::subscribe_core_notifications();
            let parking_gate = gate.clone();
            let park = tokio::spawn(CoreContext::scope(
                context,
                turn_origin::with_origin(origin("flow-live"), async move {
                    APPROVAL_FLOW_RUN_CONTEXT
                        .scope(run_context("flow-live", "run-direct"), async move {
                            parking_gate
                                .intercept_audited_for_call(
                                    "shell",
                                    "flow action",
                                    serde_json::json!({"command": "redacted"}),
                                    Some("flow-call"),
                                )
                                .await
                        })
                        .await
                }),
            ));
            let request_id = tokio::time::timeout(Duration::from_secs(5), async {
                let mut generic_request = None;
                loop {
                    match events.recv().await.unwrap() {
                        DomainEvent::ApprovalRequested {
                            request_id,
                            tool_name,
                            action_summary,
                            args_redacted,
                            thread_id,
                            client_id,
                            tool_call_id,
                            expires_at,
                            agent_id,
                        } if agent_id.as_deref() == Some("flow-agent") => {
                            assert_eq!(tool_name, "shell");
                            assert_eq!(action_summary, "flow action");
                            assert_eq!(args_redacted["command"], "redacted");
                            assert_eq!((thread_id, client_id), (None, None));
                            assert_eq!(tool_call_id.as_deref(), Some("flow-call"));
                            assert!(expires_at.is_some());
                            generic_request = Some(request_id);
                        }
                        DomainEvent::FlowApprovalRequested {
                            request_id,
                            flow_id,
                            run_id,
                            tool_name,
                            summary,
                            agent_id,
                        } if flow_id == "flow-live" => {
                            assert_eq!(generic_request.as_deref(), Some(request_id.as_str()));
                            assert_eq!(run_id, "run-direct");
                            assert_eq!(tool_name, "shell");
                            assert_eq!(summary, "flow action");
                            assert_eq!(agent_id.as_deref(), Some("flow-agent"));
                            break request_id;
                        }
                        _ => {}
                    }
                }
            })
            .await
            .expect("flow and generic approval surfaces arrive together");
            let notification = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let notification = notifications.recv().await.unwrap();
                    if notification.id == format!("flow-gate-approval:{request_id}") {
                        break notification;
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(
                notification.workspace.as_deref(),
                Some(workspace_handle.as_str())
            );
            assert!(notification.workspace_revision.is_some());
            let actions = notification.actions.unwrap();
            assert_eq!(
                actions
                    .iter()
                    .map(|action| action.action_id.as_str())
                    .collect::<Vec<_>>(),
                ["approve_once", "approve_always_for_flow", "deny"]
            );
            for action in actions {
                let payload = action.payload.unwrap();
                assert_eq!(payload["request_id"], request_id);
                assert_eq!(payload["flow_id"], "flow-live");
                assert_eq!(payload["run_id"], "run-direct");
                assert_eq!(payload["tool_name"], "shell");
                assert_eq!(payload["summary"], "flow action");
                assert_eq!(payload["decision"], action.action_id);
            }
            let rows = gate.list_pending_for_agent(Some("flow-agent")).unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].request_id, request_id);
            assert!(matches!(&rows[0].source_context,
                Some(ApprovalSourceContext::Flow { flow_id, run_id, node_id })
                    if flow_id == "flow-live" && run_id == "run-direct" && node_id.is_none()));
            assert!(gate
                .decide(&request_id, ApprovalDecision::Deny)
                .unwrap()
                .is_some());
            let (outcome, audited_request) = park.await.unwrap();
            assert!(matches!(outcome, GateOutcome::Deny { .. }));
            // Only an allowed call returns an execution-audit id; a refusal
            // correlates through the durable row and ApprovalDecided event.
            assert!(audited_request.is_none());
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let DomainEvent::ApprovalDecided {
                        request_id: decided_request,
                        tool_name,
                        decision,
                        tool_call_id,
                        agent_id,
                        ..
                    } = events.recv().await.unwrap()
                    {
                        if decided_request == request_id {
                            assert_eq!(tool_name, "shell");
                            assert_eq!(decision, "deny");
                            assert_eq!(tool_call_id.as_deref(), Some("flow-call"));
                            assert_eq!(agent_id.as_deref(), Some("flow-agent"));
                            break;
                        }
                    }
                }
            })
            .await
            .unwrap();
            assert!(gate
                .decide(&request_id, ApprovalDecision::ApproveOnce)
                .unwrap()
                .is_none());

            // Exercise the actual tool dispatcher too: a refused flow approval
            // must produce a denied tool result and never create the marker.
            let turn_agent = agent.clone();
            let turn = tokio::spawn(async move {
                APPROVAL_FLOW_RUN_CONTEXT
                    .scope(
                        run_context("flow-live", "run-tool"),
                        turn_agent.turn("write the marker").send(),
                    )
                    .await
            });
            let request = common::eventually("flow tool to park", || {
                agent.approvals().pending().ok()?.into_iter().next()
            })
            .await;
            agent
                .approvals()
                .decide(&request.request_id, ApprovalDecision::Deny)
                .unwrap();
            let outcome = turn.await.unwrap().unwrap();
            assert_eq!(outcome.reply, "flow-denied");
            assert!(!marker.exists());
            let requests = common::chat_requests(&provider).await;
            // Human approval refusals deliberately omit the policy-failure
            // marker so the model can explain that the action was not done.
            assert!(requests.iter().any(|request| common::tool_results(request)
                .contains("This action was refused and must not be performed this turn")));

            // A retained context belongs to its old instance after removal.
            // Its late flow registration must create neither rows nor surfaces.
            let _removed = runtime.agent(spec("flow-removed", "flow-closed")).unwrap();
            let old_context = AgentContextRegistry::get("flow-removed").unwrap();
            let old_scope = old_context
                .host_overrides()
                .unwrap()
                .approval_scope()
                .unwrap();
            runtime.remove_agent("flow-removed").await.unwrap();
            assert!(old_scope.is_closed());
            let mut late_events = openhuman_core::core::bus::BUS.get().unwrap().receiver();
            let mut late_notifications =
                openhuman_core::desktop::notifications::bus::subscribe_core_notifications();
            let (outcome, request) = tokio::time::timeout(
                Duration::from_secs(5),
                CoreContext::scope(
                    old_context,
                    turn_origin::with_origin(
                        origin("flow-closed"),
                        APPROVAL_FLOW_RUN_CONTEXT.scope(
                            run_context("flow-closed", "run-closed"),
                            gate.intercept_audited(
                                "shell",
                                "late flow action",
                                serde_json::json!({}),
                            ),
                        ),
                    ),
                ),
            )
            .await
            .unwrap();
            assert!(
                matches!(outcome, GateOutcome::Deny { reason } if reason.contains("agent_removed"))
            );
            assert!(request.is_none());
            assert!(gate
                .list_pending_for_agent(Some("flow-removed"))
                .unwrap()
                .is_empty());
            assert!(
                tokio::time::timeout(Duration::from_millis(100), async {
                    loop {
                        match late_events.recv().await.unwrap() {
                            DomainEvent::ApprovalRequested { agent_id, .. }
                                if agent_id.as_deref() == Some("flow-removed") =>
                            {
                                break
                            }
                            DomainEvent::FlowApprovalRequested { flow_id, .. }
                                if flow_id == "flow-closed" =>
                            {
                                break
                            }
                            _ => {}
                        }
                    }
                })
                .await
                .is_err(),
                "removed instance published an approval surface"
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(100), async {
                    loop {
                        let notification = late_notifications.recv().await.unwrap();
                        if notification.actions.as_ref().is_some_and(|actions| {
                            actions.iter().any(|action| {
                                action
                                    .payload
                                    .as_ref()
                                    .is_some_and(|payload| payload["flow_id"] == "flow-closed")
                            })
                        }) {
                            break;
                        }
                    }
                })
                .await
                .is_err(),
                "removed instance published an actionable notification"
            );
        })
        .await
        .unwrap();
    });
}
