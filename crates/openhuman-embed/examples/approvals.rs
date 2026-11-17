//! Title: Polling and callback approvals with cancellation
//! Summary: Allow and deny real parked writes, then cancel an in-flight callback without losing its pending request.
//! Run: offline with loopback stubs; no live path.
//! Feature: default

mod support;
use openhuman_embed::{AgentDefinitionSpec, AgentSpec, Runtime, ToolScopeSpec, Workspace};

fn main() -> anyhow::Result<()> {
    support::run(run())
}

async fn run() -> anyhow::Result<()> {
    let backend = support::stub_backend().await;
    let provider = support::provider("hello from the stub").await;
    let mut config = support::offline_config();
    config.autonomy.enabled = true;
    let runtime = Runtime::builder()
        .config(config)
        .workspace(Workspace::Ephemeral)
        .backend_url(backend.uri())
        .provider(support::route(&provider, "fixture"))
        .build()
        .await?;
    // ANCHOR: approvals
    let scratch = tempfile::tempdir()?;
    let marker = scratch.path().join("approved-marker");
    let write_provider = support::scripted_provider(
        vec![support::tool_call_completion(
            "shell",
            &serde_json::json!({"command": format!("touch {}", marker.display())}).to_string(),
        )],
        "approved",
    )
    .await;
    let agent = runtime.agent(
        AgentSpec::new("supervised")
            .provider(support::route(&write_provider, "fixture"))
            .action_dir(scratch.path())
            .access(
                openhuman_embed::Access::supervised()
                    .origin(openhuman_embed::AgentTurnOrigin::WebChat {
                        thread_id: "demo".into(),
                        client_id: "host".into(),
                        request_id: None,
                    })
                    .auto_approve(Vec::<String>::new())
                    .auto_approve_all(false)
                    .trust(
                        scratch.path().display().to_string(),
                        openhuman_embed::TrustedAccess::ReadWrite,
                    ),
            )
            .definition(
                AgentDefinitionSpec::new().tools(ToolScopeSpec::Named(vec!["shell".into()])),
            ),
    )?;
    let pending_agent = agent.clone();
    let turn = tokio::spawn(async move { pending_agent.run("Create a marker").await });
    let pending = support::eventually("approval", || {
        agent.approvals().pending().ok()?.into_iter().next()
    })
    .await;
    assert!(!marker.exists(), "execution waits for the host decision");
    assert_eq!(pending.agent_id.as_deref(), Some("supervised"));
    agent.approvals().decide(
        &pending.request_id,
        openhuman_embed::ApprovalDecision::ApproveOnce,
    )?;
    assert_eq!(turn.await??.reply, "approved");
    assert!(marker.exists());
    assert!(agent.approvals().pending()?.is_empty());
    println!("parked write executed only after approval");
    // ANCHOR_END: approvals
    // ANCHOR: approval_callback
    let denied_marker = scratch.path().join("denied-marker");
    let denied_provider = support::scripted_provider(
        vec![support::tool_call_completion(
            "shell",
            &serde_json::json!({"command": format!("touch {}", denied_marker.display())})
                .to_string(),
        )],
        "denied",
    )
    .await;
    let decisions = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let callback_agent = runtime.agent(
        AgentSpec::new("callback")
            .provider(support::route(&denied_provider, "fixture"))
            .action_dir(scratch.path())
            .access(
                openhuman_embed::Access::supervised()
                    .origin(openhuman_embed::AgentTurnOrigin::WebChat {
                        thread_id: "callback".into(),
                        client_id: "host".into(),
                        request_id: None,
                    })
                    .auto_approve(Vec::<String>::new())
                    .auto_approve_all(false)
                    .trust(
                        scratch.path().display().to_string(),
                        openhuman_embed::TrustedAccess::ReadWrite,
                    ),
            )
            .definition(
                AgentDefinitionSpec::new().tools(ToolScopeSpec::Named(vec!["shell".into()])),
            )
            .approval_handler(std::sync::Arc::new(DenyWrites(decisions.clone()))),
    )?;
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        callback_agent.run("Create a marker"),
    )
    .await??;
    assert_eq!(outcome.reply, "denied");
    assert_eq!(decisions.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(!denied_marker.exists());
    // Callback allowance uses the same gate and executes the real write.
    let allowed_marker = scratch.path().join("callback-allowed");
    let allowed_provider = writing_provider(&allowed_marker, "callback approved").await;
    let allowed_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let allowed = runtime.agent(
        supervised_spec("allowed", scratch.path(), &allowed_provider)
            .approval_handler(std::sync::Arc::new(AllowWrites(allowed_calls.clone()))),
    )?;
    assert_eq!(
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            allowed.run("Create a marker")
        )
        .await??
        .reply,
        "callback approved"
    );
    assert!(allowed_marker.exists());
    assert_eq!(allowed_calls.load(std::sync::atomic::Ordering::SeqCst), 1);

    // A temporary callback may be removed while its decision is in flight.
    // Channels prove both startup and future destruction without timing sleeps.
    let cancelled_marker = scratch.path().join("callback-cancelled");
    let cancelled_provider = writing_provider(&cancelled_marker, "cancelled decision").await;
    let cancel_agent = runtime.agent(supervised_spec(
        "cancel-callback",
        scratch.path(),
        &cancelled_provider,
    ))?;
    let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
    let (dropped_tx, mut dropped_rx) = tokio::sync::mpsc::unbounded_channel();
    let subscription = cancel_agent.handle_approvals(std::sync::Arc::new(HangingDecision {
        started: started_tx,
        dropped: dropped_tx,
    }));
    let turning = cancel_agent.clone();
    let cancelled_turn = tokio::spawn(async move { turning.run("Create a marker").await });
    let request_id = tokio::time::timeout(std::time::Duration::from_secs(10), started_rx.recv())
        .await?
        .expect("callback started");
    drop(subscription);
    tokio::time::timeout(std::time::Duration::from_secs(10), dropped_rx.recv())
        .await?
        .expect("decision future dropped");
    assert!(!cancelled_marker.exists());
    assert!(cancel_agent
        .approvals()
        .pending()?
        .iter()
        .any(|request| request.request_id == request_id));
    cancel_agent
        .approvals()
        .decide(&request_id, openhuman_embed::ApprovalDecision::Deny)?;
    assert_eq!(cancelled_turn.await??.reply, "cancelled decision");
    assert!(!cancelled_marker.exists());
    // ANCHOR_END: approval_callback
    support::passed("approvals");
    Ok(())
}

struct DenyWrites(std::sync::Arc<std::sync::atomic::AtomicUsize>);
#[async_trait::async_trait]
impl openhuman_embed::ApprovalHandler for DenyWrites {
    async fn decide(
        &self,
        request: &openhuman_embed::PendingApproval,
    ) -> openhuman_embed::ApprovalDecision {
        assert_eq!(request.agent_id.as_deref(), Some("callback"));
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        openhuman_embed::ApprovalDecision::Deny
    }
}

async fn writing_provider(marker: &std::path::Path, reply: &str) -> wiremock::MockServer {
    support::scripted_provider(
        vec![support::tool_call_completion(
            "shell",
            &serde_json::json!({"command":format!("touch {}", marker.display())}).to_string(),
        )],
        reply,
    )
    .await
}
fn supervised_spec(
    id: &str,
    scratch: &std::path::Path,
    provider: &wiremock::MockServer,
) -> AgentSpec {
    AgentSpec::new(id)
        .provider(support::route(provider, "fixture"))
        .action_dir(scratch)
        .access(
            openhuman_embed::Access::supervised()
                .origin(openhuman_embed::AgentTurnOrigin::WebChat {
                    thread_id: id.into(),
                    client_id: "host".into(),
                    request_id: None,
                })
                .auto_approve(Vec::<String>::new())
                .auto_approve_all(false)
                .trust(
                    scratch.display().to_string(),
                    openhuman_embed::TrustedAccess::ReadWrite,
                ),
        )
        .definition(AgentDefinitionSpec::new().tools(ToolScopeSpec::Named(vec!["shell".into()])))
}
struct AllowWrites(std::sync::Arc<std::sync::atomic::AtomicUsize>);
#[async_trait::async_trait]
impl openhuman_embed::ApprovalHandler for AllowWrites {
    async fn decide(
        &self,
        request: &openhuman_embed::PendingApproval,
    ) -> openhuman_embed::ApprovalDecision {
        assert_eq!(request.agent_id.as_deref(), Some("allowed"));
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        openhuman_embed::ApprovalDecision::ApproveOnce
    }
}
struct HangingDecision {
    started: tokio::sync::mpsc::UnboundedSender<String>,
    dropped: tokio::sync::mpsc::UnboundedSender<()>,
}
struct OnDecisionDrop(tokio::sync::mpsc::UnboundedSender<()>);
impl Drop for OnDecisionDrop {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}
#[async_trait::async_trait]
impl openhuman_embed::ApprovalHandler for HangingDecision {
    async fn decide(
        &self,
        request: &openhuman_embed::PendingApproval,
    ) -> openhuman_embed::ApprovalDecision {
        let _guard = OnDecisionDrop(self.dropped.clone());
        let _ = self.started.send(request.request_id.clone());
        std::future::pending().await
    }
}
