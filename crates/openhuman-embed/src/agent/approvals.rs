//! One agent's view of the runtime's approval gate.
//!
//! The gate and its `pending_approvals` store are process-owned, shared by
//! every agent on the runtime. Each request records the agent that parked it,
//! and an [`Approvals`] handle lists and decides only its own agent's.

use openhuman_core::security::approval::{ApprovalError, ApprovalGate};

pub use openhuman_core::security::approval::{ApprovalDecision, PendingApproval};

/// Why an [`Approvals`] call failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApprovalsError {
    /// The runtime runs without an approval gate, so nothing ever parks.
    #[error("the approval gate is not installed on this runtime")]
    GateNotInstalled,
    /// The request was parked by another agent.
    #[error("approval request {0} belongs to another agent")]
    WrongAgent(String),
    /// No undecided request with this id exists for the agent.
    #[error("no pending approval {0} for this agent")]
    NotFound(String),
    /// The approval store failed.
    #[error("approval store: {0}")]
    Store(String),
}

/// The pending approvals of one agent. Obtain with
/// [`Agent::approvals`](super::Agent::approvals).
#[derive(Debug, Clone)]
pub struct Approvals {
    agent_id: String,
    removed: tokio::sync::watch::Receiver<bool>,
    lifecycle: std::sync::Arc<super::lifecycle::ApprovalState>,
}

impl Approvals {
    pub(crate) fn new(
        agent_id: &str,
        removed: tokio::sync::watch::Receiver<bool>,
        lifecycle: std::sync::Arc<super::lifecycle::ApprovalState>,
    ) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            removed,
            lifecycle,
        }
    }

    /// The agent's undecided requests, oldest first. Empty when the runtime
    /// has no approval gate or this agent has been removed. Reusing the id
    /// does not make a removed agent's handle observe the replacement.
    pub fn pending(&self) -> Result<Vec<PendingApproval>, ApprovalsError> {
        // Hold the lifecycle read through the gate access. Removal must finish
        // before the id can be reused, so a concurrent old handle cannot cross
        // from its lifecycle check into the replacement's approval store.
        let removed = self.removed.borrow();
        if *removed {
            return Ok(Vec::new());
        }
        let Some(gate) = ApprovalGate::try_global() else {
            return Ok(Vec::new());
        };
        gate.list_pending_for_agent(Some(&self.agent_id))
            .map_err(|err| ApprovalsError::Store(err.to_string()))
    }

    /// Decide one of the agent's requests and release its parked call.
    ///
    /// [`ApprovalDecision::ApproveAlwaysForTool`] approves this call only; an
    /// agent's standing grants come from
    /// [`Access::auto_approve`](crate::Access::auto_approve).
    pub fn decide(
        &self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<PendingApproval, ApprovalsError> {
        self.lifecycle
            .with_live(|| self.decide_live(request_id, decision))
            .unwrap_or_else(|| Err(ApprovalsError::NotFound(request_id.to_owned())))
    }

    fn decide_live(
        &self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<PendingApproval, ApprovalsError> {
        let removed = self.removed.borrow();
        if *removed {
            return Err(ApprovalsError::NotFound(request_id.to_owned()));
        }
        let gate = ApprovalGate::try_global().ok_or(ApprovalsError::GateNotInstalled)?;
        log::debug!(
            "[embed][approvals] agent={} decide request_id={request_id} decision={}",
            self.agent_id,
            decision.as_str()
        );
        match gate.decide_for_agent(&self.agent_id, request_id, decision) {
            Ok(Some(row)) => Ok(row),
            Ok(None) => Err(ApprovalsError::NotFound(request_id.to_string())),
            Err(err) => match err.downcast_ref::<ApprovalError>() {
                Some(ApprovalError::WrongAgent { request_id }) => {
                    Err(ApprovalsError::WrongAgent(request_id.clone()))
                }
                _ => Err(ApprovalsError::Store(err.to_string())),
            },
        }
    }
}
