//! The approval-by-reply intercept: which channel instances may decide a
//! parked approval with a chat reply, and which replies count.

use super::consumes_approval_reply;
use crate::channels::Channel;
use crate::core::runtime::turn_origin::{self, AgentTurnOrigin};
use crate::security::approval::{
    ApprovalChatContext, ApprovalGate, ApprovalTurnContext, GateOutcome,
};
use std::sync::Arc;
use tinychannels_bus as traits;

struct Fake {
    chat_approvals: bool,
}

#[async_trait::async_trait]
impl Channel for Fake {
    fn name(&self) -> &str {
        "fake"
    }
    fn capabilities(&self) -> traits::ChannelCapabilities {
        let mut capabilities = traits::ChannelCapabilities::default();
        capabilities.chat_approvals = self.chat_approvals;
        capabilities
    }
    #[allow(
        deprecated,
        reason = "TinyChannels v22 still requires the legacy send trait method"
    )]
    async fn send(&self, _message: &traits::SendMessage) -> anyhow::Result<()> {
        Ok(())
    }
    #[allow(
        deprecated,
        reason = "TinyChannels v22 still requires the legacy listener trait method"
    )]
    async fn listen(
        &self,
        _tx: tokio::sync::mpsc::Sender<traits::ChannelMessage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

fn channel(chat_approvals: bool) -> Arc<dyn Channel> {
    Arc::new(Fake { chat_approvals })
}

fn reply(text: &str) -> traits::ChannelInboundEnvelope {
    traits::ChannelInboundEnvelope {
        text: text.into(),
        ..Default::default()
    }
}

/// Park a tool call on `thread`; returns its request id once it is routed, and
/// the parked call's task.
async fn park(
    gate: &Arc<ApprovalGate>,
    thread: &str,
) -> (
    String,
    tokio::task::JoinHandle<(GateOutcome, Option<String>)>,
) {
    let (g, t) = (Arc::clone(gate), thread.to_string());
    let parked = tokio::spawn(async move {
        let origin = AgentTurnOrigin::WebChat {
            thread_id: t.clone(),
            client_id: "c".into(),
            request_id: None,
        };
        let chat = ApprovalChatContext {
            thread_id: t,
            client_id: "c".into(),
            request_id: None,
        };
        turn_origin::with_origin(
            origin,
            g.intercept_audited(
                &ApprovalTurnContext::chat(chat),
                "shell",
                "run it",
                serde_json::json!({}),
            ),
        )
        .await
    });
    for _ in 0..2_000 {
        if let Some(request_id) = gate.pending_for_thread(thread) {
            return (request_id, parked);
        }
        tokio::task::yield_now().await;
    }
    panic!("no approval parked on {thread}");
}

#[tokio::test]
async fn only_a_chat_approvals_channel_decides_a_parked_approval() {
    let dir = tempfile::tempdir().unwrap();
    let gate = Arc::new(ApprovalGate::for_test(dir.path()));
    let (request_id, parked) = park(&gate, "t-chan").await;
    let yes = reply("yes");

    // Fails closed: no channel instance, or one without chat approvals.
    assert!(!consumes_approval_reply(None, Some(&gate), &yes, "t-chan"));
    let off = channel(false);
    assert!(!consumes_approval_reply(
        Some(&off),
        Some(&gate),
        &yes,
        "t-chan"
    ));
    // No gate installed, a reply that is not yes/no, or another thread.
    let on = channel(true);
    assert!(!consumes_approval_reply(Some(&on), None, &yes, "t-chan"));
    assert!(!consumes_approval_reply(
        Some(&on),
        Some(&gate),
        &reply("hello"),
        "t-chan"
    ));
    assert!(!consumes_approval_reply(
        Some(&on),
        Some(&gate),
        &yes,
        "t-other"
    ));
    assert_eq!(gate.pending_for_thread("t-chan"), Some(request_id));

    // A chat-approvals channel decides it, once.
    assert!(consumes_approval_reply(
        Some(&on),
        Some(&gate),
        &yes,
        "t-chan"
    ));
    let (outcome, _) = parked.await.unwrap();
    assert!(matches!(outcome, GateOutcome::Allow), "{outcome:?}");
    assert!(gate.pending_for_thread("t-chan").is_none());
    assert!(!consumes_approval_reply(
        Some(&on),
        Some(&gate),
        &yes,
        "t-chan"
    ));
}
