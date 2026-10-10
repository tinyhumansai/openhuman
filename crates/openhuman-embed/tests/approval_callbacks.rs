//! Callback approvals use the same instance-owned gate as polling approvals.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{
    offline_config, route, runtime, scripted_provider, stub_backend, tool_call_completion,
};
use openhuman_core::security::AutonomyLevel;
use openhuman_embed::{
    Access, AgentDefinitionSpec, AgentSpec, AgentTurnOrigin, ApprovalDecision, ApprovalHandler,
    PendingApproval, Runtime, ToolScopeSpec, TrustedAccess, Workspace,
};

struct ApproveAndRecord(tokio::sync::mpsc::UnboundedSender<PendingApproval>);

#[async_trait::async_trait]
impl ApprovalHandler for ApproveAndRecord {
    async fn decide(&self, request: &PendingApproval) -> ApprovalDecision {
        self.0
            .send(request.clone())
            .expect("approval receiver remains live");
        ApprovalDecision::ApproveOnce
    }
}

#[test]
fn callback_receives_the_owned_request_and_releases_its_parked_tool() {
    runtime().block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let backend = stub_backend().await;
    let scratch = tempfile::tempdir().unwrap();
    let marker = scratch.path().join("approved-tool");
    let provider = scripted_provider(
        vec![tool_call_completion(
            "shell",
            &serde_json::json!({"command": format!("touch {}", marker.display())}).to_string(),
        )],
        "approved reply",
    )
    .await;
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
        .unwrap();
    let (requests, mut received) = tokio::sync::mpsc::unbounded_channel();
    let agent = runtime
        .agent(
            AgentSpec::new("callback-worker")
                .provider(route(&provider, "callback-model"))
                .access(
                    Access::supervised()
                        .origin(AgentTurnOrigin::WebChat {
                            thread_id: "callback-thread".into(),
                            client_id: "callback-client".into(),
                            request_id: None,
                        })
                        .auto_approve(Vec::<String>::new())
                        .auto_approve_all(false)
                        .trust(
                            scratch.path().display().to_string(),
                            TrustedAccess::ReadWrite,
                        ),
                )
                .definition(
                    AgentDefinitionSpec::new().tools(ToolScopeSpec::Named(vec!["shell".into()])),
                )
                .approval_handler(Arc::new(ApproveAndRecord(requests))),
        )
        .unwrap();
    let turn = tokio::spawn(agent.turn("Run the approved shell operation").send());
    let request = tokio::time::timeout(Duration::from_secs(10), received.recv())
        .await
        .expect("the callback receives the parked request")
        .expect("approval channel stays live");
    assert_eq!(request.agent_id.as_deref(), Some("callback-worker"));
    assert_eq!(request.tool_name, "shell");
    assert!(!request.request_id.is_empty());
    let outcome = tokio::time::timeout(Duration::from_secs(10), turn)
        .await
        .expect("callback decision releases the turn")
        .unwrap()
        .unwrap();
    assert_eq!(outcome.reply, "approved reply");
    assert!(marker.exists(), "the approved tool actually executed");
    assert!(agent.approvals().pending().unwrap().is_empty());
    runtime.remove_agent("callback-worker").await.unwrap();
    assert!(agent.approvals().pending().unwrap().is_empty());
    assert!(matches!(
        agent.turn("after removal").send().await,
        Err(openhuman_embed::CoreError::AgentRemoved { .. })
    ));
}
