//! Callback-based approvals, using the same agent-scoped gate as polling.
use crate::{ApprovalDecision, Approvals, CancellationToken, PendingApproval};
use openhuman_core::core::bus::{EventHandler, SubscriptionHandle, BUS};
use openhuman_core::core::events::DomainEvent;
use std::sync::Arc;

/// Decide an agent's sanitized pending permission request asynchronously.
#[async_trait::async_trait]
pub trait ApprovalHandler: Send + Sync {
    /// Return a decision; removing the agent or subscription cancels an in-flight callback.
    async fn decide(&self, request: &PendingApproval) -> ApprovalDecision;
}

/// Owns a live callback subscription; dropping it or removing its agent cancels callbacks.
pub struct ApprovalSubscription {
    _subscription: Option<SubscriptionHandle>,
    cancellation: CancellationToken,
}
impl Drop for ApprovalSubscription {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}
impl ApprovalSubscription {
    pub(crate) fn new(
        agent_id: &str,
        handler: Arc<dyn ApprovalHandler>,
        removed: tokio::sync::watch::Receiver<bool>,
        lifecycle: Arc<super::lifecycle::ApprovalState>,
    ) -> Self {
        let cancellation = CancellationToken::new();
        let subscriber = Arc::new(Handler {
            agent_id: agent_id.to_owned(),
            handler,
            cancellation: cancellation.clone(),
            removed,
            lifecycle,
        });
        Self {
            _subscription: BUS.subscribe(subscriber),
            cancellation,
        }
    }
}
struct Handler {
    agent_id: String,
    handler: Arc<dyn ApprovalHandler>,
    cancellation: CancellationToken,
    removed: tokio::sync::watch::Receiver<bool>,
    lifecycle: Arc<super::lifecycle::ApprovalState>,
}
#[async_trait::async_trait]
impl EventHandler<DomainEvent> for Handler {
    fn name(&self) -> &str {
        "embed.agent.approval_handler"
    }
    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::ApprovalRequested {
            request_id,
            agent_id,
            ..
        } = event
        else {
            return;
        };
        if agent_id.as_deref() != Some(&self.agent_id) || *self.removed.borrow() {
            return;
        }
        let approvals =
            Approvals::new(&self.agent_id, self.removed.clone(), self.lifecycle.clone());
        let Ok(pending) = approvals.pending() else {
            return;
        };
        let Some(request) = pending
            .into_iter()
            .find(|request| request.request_id == *request_id)
        else {
            return;
        };
        let mut removed = self.removed.clone();
        let decision = tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => return,
            _ = removed.wait_for(|removed| *removed) => return,
            decision = self.handler.decide(&request) => decision,
        };
        if self.cancellation.is_cancelled() || *self.removed.borrow() {
            return;
        }
        // The gate atomically refuses a request that expired/was removed while
        // the callback ran, so a late answer never revives a cancelled tool.
        let _ = approvals.decide(request_id, decision);
    }
}
