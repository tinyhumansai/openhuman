//! The turn's approval context reaches the gate through the harness security
//! gate (`OpenHumanSecurityGate`), carried by the run context rather than any
//! ambient scope; a detached turn starts without it; concurrent turns keep
//! their own.

use super::*;
use crate::agent::tinyagents::host::{OpenHumanRunContext, OpenHumanSecurityGate};
use crate::security::policy::SecurityPolicy;
use async_trait::async_trait;
use tinyagents_harness::host::security_gate::{GateDecision, ToolCallRequest};
use tinytools::{PermissionLevel, Tool, ToolResult};

struct Effectful;

#[async_trait]
impl Tool for Effectful {
    fn name(&self) -> &str {
        "slack_post"
    }
    fn description(&self) -> &str {
        "test tool with an external effect"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    fn external_effect(&self) -> bool {
        true
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        unreachable!("the security gate never executes a tool")
    }
}

fn security_gate_for(run: &OpenHumanRunContext) -> OpenHumanSecurityGate {
    OpenHumanSecurityGate::new(
        std::sync::Arc::new(SecurityPolicy::default()),
        vec![std::sync::Arc::new(vec![
            Box::new(Effectful) as Box<dyn Tool>
        ])],
    )
    .with_approval(run.approval.clone())
}

fn chat(thread: &str) -> ApprovalChatContext {
    ApprovalChatContext {
        thread_id: thread.to_string(),
        client_id: format!("client-{thread}"),
        request_id: Some(format!("req-{thread}")),
    }
}

fn web_origin_for(thread: &str) -> AgentTurnOrigin {
    AgentTurnOrigin::WebChat {
        thread_id: thread.to_string(),
        client_id: format!("client-{thread}"),
        request_id: Some(format!("req-{thread}")),
    }
}

fn call() -> ToolCallRequest {
    ToolCallRequest::new("slack_post", serde_json::json!({"text": "hi"}), "lead")
}

/// Waits for the thread's park to register, returning its request id.
async fn parked(gate: &ApprovalGate, thread: &str) -> String {
    for _ in 0..2_000 {
        if let Some(request_id) = gate.pending_for_thread(thread) {
            return request_id;
        }
        tokio::task::yield_now().await;
    }
    panic!("no approval parked for thread {thread}");
}

#[tokio::test]
async fn a_turn_parks_its_approval_on_the_chat_its_run_context_carries() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let mut run = OpenHumanRunContext::new();
    run.approval = ApprovalTurnContext::chat(chat("t-turn"));
    let security = security_gate_for(&run);

    let g = gate.clone();
    let task = tokio::spawn(async move {
        turn_origin::with_origin(web_origin_for("t-turn"), async move {
            // A real await point before the gate check: the context must survive it.
            tokio::task::yield_now().await;
            security.park_for_approval_on(&g, &call()).await
        })
        .await
    });

    let request_id = parked(&gate, "t-turn").await;
    // A park with a chat context is attached to the turn, not a detached card.
    assert!(!gate.request_is_detached(&request_id));
    gate.decide(&request_id, ApprovalDecision::ApproveOnce)
        .unwrap();
    assert!(matches!(
        task.await.unwrap(),
        GateDecision::Prompted { approved: true }
    ));
}

#[tokio::test]
async fn a_detached_turn_has_no_chat_and_routes_through_its_origin_marked_detached() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let mut parent = OpenHumanRunContext::new();
    parent.approval = ApprovalTurnContext::chat(chat("t-parent"));
    // What an async-delegated sub-agent runs with (#5499).
    let detached = parent.detached_child();
    let security = security_gate_for(&detached);

    let g = gate.clone();
    let task = tokio::spawn(async move {
        turn_origin::with_origin(web_origin_for("t-parent"), async move {
            security.park_for_approval_on(&g, &call()).await
        })
        .await
    });

    let request_id = parked(&gate, "t-parent").await;
    assert!(
        gate.request_is_detached(&request_id),
        "a detached sub-agent's park must not inherit the parent's chat context"
    );
    gate.decide(&request_id, ApprovalDecision::ApproveOnce)
        .unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn concurrent_turns_park_on_their_own_chats() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let mut tasks = Vec::new();
    for thread in ["t-one", "t-two"] {
        let mut run = OpenHumanRunContext::new();
        run.approval = ApprovalTurnContext::chat(chat(thread));
        let security = security_gate_for(&run);
        let g = gate.clone();
        tasks.push(tokio::spawn(async move {
            turn_origin::with_origin(web_origin_for(thread), async move {
                security.park_for_approval_on(&g, &call()).await
            })
            .await
        }));
    }
    for thread in ["t-one", "t-two"] {
        let request_id = parked(&gate, thread).await;
        gate.decide(&request_id, ApprovalDecision::ApproveOnce)
            .unwrap();
    }
    for task in tasks {
        assert!(matches!(
            task.await.unwrap(),
            GateDecision::Prompted { approved: true }
        ));
    }
}
