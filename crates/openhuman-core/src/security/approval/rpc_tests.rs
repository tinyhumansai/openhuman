use super::*;

use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;
use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet, Tenant};
use crate::security::approval::engine_stub::StubEngine;
use crate::security::approval::{ApprovalChatContext, ApprovalTurnContext, GateOutcome};
use crate::security::turn_origin::{self, AgentTurnOrigin};
use crate::tools::toolpacks::ToolGroups;
use tempfile::TempDir;

fn fixture() -> (
    Arc<ApprovalGate>,
    Arc<StubEngine>,
    TempDir,
    Arc<CoreContext>,
) {
    let dir = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let engine = StubEngine::new();
    let gate = Arc::new(ApprovalGate::with_engine(
        config.clone(),
        format!("session-{}", uuid::Uuid::new_v4()),
        None,
        engine.clone(),
    ));
    let root = CoreContext::for_test(DomainSet::full(), Some(dir.path().to_path_buf()));
    let ctx = root.derive_with(
        ContextOverlay::new(config, DomainSet::kernel(), ToolGroups::none())
            .session_agent("approval-agent"),
    );
    (gate, engine, dir, ctx)
}

fn chat_origin() -> AgentTurnOrigin {
    AgentTurnOrigin::WebChat {
        thread_id: "approval-thread".into(),
        client_id: "approval-client".into(),
        request_id: Some("approval-turn".into()),
    }
}

async fn park(
    gate: Arc<ApprovalGate>,
    ctx: Arc<CoreContext>,
) -> (String, tokio::task::JoinHandle<GateOutcome>) {
    let call_gate = Arc::clone(&gate);
    let waiting = tokio::spawn(CoreContext::scope(
        ctx,
        turn_origin::with_origin(chat_origin(), async move {
            call_gate
                .intercept(
                    &ApprovalTurnContext::chat(ApprovalChatContext {
                        thread_id: "approval-thread".into(),
                        client_id: "approval-client".into(),
                        request_id: None,
                    }),
                    "shell",
                    "run it",
                    serde_json::json!({}),
                )
                .await
        }),
    ));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(request_id) =
            gate.pending_for_agent_thread(Some("approval-agent"), "approval-thread")
        {
            return (request_id, waiting);
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "approval did not park"
        );
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn current_call_resolves_when_saving_a_whole_tool_grant_fails() {
    let (gate, engine, _dir, ctx) = fixture();
    let (request_id, waiting) = park(Arc::clone(&gate), ctx).await;
    let scope = Tenant {
        profile: None,
        agent: Some("approval-agent".into()),
    };

    // Decide first. The parked call is released before the best-effort durable
    // grant runs; a persistence failure can affect only subsequent calls.
    let row = gate
        .decide(&request_id, ApprovalDecision::ApproveAlwaysForTool)
        .unwrap()
        .expect("the parked request was decided");
    engine.refuse_checks(true);
    let warning = persist_tool_grant_after_decision(
        &gate,
        ApprovalDecision::ApproveAlwaysForTool,
        (scope, Variant::Standard),
        &request_id,
        &row.tool_name,
    )
    .expect("persistence failure should be surfaced as a warning");

    assert!(warning.contains("WARNING: could not save 'Always allow'"));
    assert!(matches!(waiting.await.unwrap(), GateOutcome::Allow));
}

#[tokio::test]
async fn whole_tool_grant_persists_only_after_the_current_call_is_approved() {
    let (gate, _engine, _dir, ctx) = fixture();
    let (request_id, waiting) = park(Arc::clone(&gate), ctx).await;
    let scope = Tenant {
        profile: None,
        agent: Some("approval-agent".into()),
    };

    let row = gate
        .decide(&request_id, ApprovalDecision::ApproveAlwaysForTool)
        .unwrap()
        .expect("the parked request was decided");
    let log = persist_tool_grant_after_decision(
        &gate,
        ApprovalDecision::ApproveAlwaysForTool,
        (scope, Variant::Standard),
        &request_id,
        &row.tool_name,
    )
    .expect("a standard scope should persist its grant");

    assert!(log.contains("added to the Always-allow list"));
    assert!(matches!(waiting.await.unwrap(), GateOutcome::Allow));
}
