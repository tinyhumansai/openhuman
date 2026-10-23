//! The approval engine the gate asks, and the prompt surface the engine calls.
//!
//! The durable lifecycle of an approval (pending, approved, denied, expired,
//! cancelled, reserved, executed), its exact binding to the call, the tool and
//! flow grants, the 10-minute interactive expiry and the audit trail are the
//! TinySecurity module's. The gate keeps what is the host's: the verified
//! origin of a call, the approval posture, the legacy import payload, the parked
//! tool-call future, chat routing and the product events.
//!
//! This file is the seam between the two. It holds no module types, so the gate
//! compiles in a build without the module; the only implementations are the
//! native one (`native/`, behind `security-module`), which asks the module over
//! its bus contract, and the unavailable one a slim build gets, which refuses
//! everything. The trait is sealed: nothing outside this crate can name an
//! implementation, so a holder of an [`ApprovalGate`](super::ApprovalGate)
//! cannot swap in an allow-everything engine.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::core::runtime::Tenant;

mod sealed {
    /// Only this crate can implement [`super::ApprovalEngine`].
    pub trait Sealed {}
}

pub(crate) use sealed::Sealed;

/// Which approval posture a call is evaluated under. A variant is a separate
/// registered subject, because the module keeps one posture per subject.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Variant {
    /// The agent's own approvals: its allowlist, its flow trust, its auto-approve
    /// setting.
    Standard,
    /// A workflow run whose flow has `require_approval` set. The global tool
    /// allowlist must never approve it; per-flow trust still does.
    FlowStrict,
    /// A one-time decision for this exact action: no grant, no auto-approval,
    /// whatever the user configured.
    Forced,
}

impl Variant {
    /// The suffix that makes the variant's subject distinct from the agent's.
    pub(crate) fn suffix(self) -> &'static str {
        match self {
            Self::Standard => "",
            Self::FlowStrict => "#flow-strict",
            Self::Forced => "#forced",
        }
    }
}

/// Runtime approval settings registered for a subject; durable tool and flow
/// grants are kept by the module and are not part of this posture.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Posture {
    /// An approval is required unless the agent's approvals are off.
    pub require_approval: bool,
    /// The user opted into approving everything.
    pub auto_approve_all: bool,
}

/// The verified origin class the module sees. The module, not the host, decides
/// what each class is allowed (contract 1.22): the gate only maps the turn's
/// origin to one of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CallOrigin {
    /// A live user, a channel peer or a workflow run.
    Operator,
    /// The Flow Canvas copilot's streaming turn (shorter park window).
    Copilot,
    /// A delegated sub-agent (shorter park window).
    SubAgent,
    /// A scheduled job: the module denies its external effects.
    Cron,
    /// An internal background job or a workflow action the user already
    /// declared: the module approves it automatically, audited.
    TrustedAutomation,
    /// A local operator invoking the core directly (the CLI, a direct chat): the
    /// module approves it automatically, audited.
    LocalOperator,
    /// A call whose origin nobody labelled: the module denies it unless the
    /// durable whole-tool trust names the tool.
    Unlabelled,
}

/// One gated call, as the gate hands it over.
pub(crate) struct CheckInput {
    /// The tenant whose subject and storage scope the call runs under.
    pub tenant: Tenant,
    /// The posture variant.
    pub variant: Variant,
    /// The user's configured posture for this subject, read live.
    pub posture: Posture,
    /// Legacy `auto_approve` entries carried only so the native engine can
    /// import them once as durable module trust; they are not runtime posture.
    pub legacy_tool_grants: Vec<String>,
    /// The correlation the prompt surface finds the call by. Eight lowercase hex
    /// characters, which the module's identifier hashing leaves unchanged.
    pub call_id: String,
    /// The gated tool's name.
    pub tool: String,
    /// The call's arguments, already redacted by the host; always an object.
    pub args: Value,
    /// A short summary shown to the user.
    pub summary: String,
    /// The conversation thread the call runs in.
    pub thread: Option<String>,
    /// The workflow the call runs for.
    pub flow: Option<String>,
    /// The origin class.
    pub origin: CallOrigin,
    /// A shorter park window than the module's own (ten minutes; three for a
    /// copilot or sub-agent), when the user configured one. `None` leaves the
    /// window to the module, which only ever shortens it.
    pub ttl: Option<Duration>,
    /// Where the engine shows the prompt when a human must decide.
    pub prompt: Arc<dyn PromptSurface>,
}

/// What the engine answered for a gated call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Checked {
    /// An audited automatic approval (a grant or the auto-approve posture). The
    /// caller still reserves it before executing.
    Approved { request_id: String },
    /// A human must decide. The prompt was delivered to the [`PromptSurface`].
    Pending {
        request_id: String,
        /// When the request expires, in epoch milliseconds.
        expires_at_ms: u64,
    },
}

/// A human's choice, after the host authenticated them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    /// Run this call.
    Once,
    /// Refuse it.
    Deny,
}

/// What a decision left the request as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decided {
    /// Approved; the caller reserves the call before it runs.
    Approved,
    /// Denied.
    Denied,
}

/// How the engine says no.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum EngineError {
    /// The request is no longer undecided: it was decided, expired, cancelled or
    /// never existed in this process. A benign race for a decision.
    #[error("the approval request is not pending")]
    NotPending,
    /// The module answered no: a scheduled job's external effect, an unlabelled
    /// call no grant covers, or any other denial it will not let a person
    /// override. Nothing was parked.
    #[error("the approval engine refused the call")]
    Refused,
    /// The module or its store could not answer. Fail closed.
    #[error("{0}")]
    Unavailable(String),
}

/// The part of the approval lifecycle the gate drives.
#[async_trait]
pub(crate) trait ApprovalEngine: sealed::Sealed + std::fmt::Debug + Send + Sync {
    /// Evaluate a gated call: register or refresh its subject, then ask the
    /// engine.
    async fn check(&self, input: CheckInput) -> Result<Checked, EngineError>;

    /// Apply a human's decision to a pending request. `Err(NotPending)` when the
    /// request is not pending in this process.
    /// `label` is what the user chose in the host's own words (for example
    /// `approve_always_for_tool`), kept beside the row for the history view.
    async fn decide(
        &self,
        request_id: &str,
        choice: Choice,
        label: &str,
    ) -> Result<Decided, EngineError>;

    /// Reserve an approved request for execution, exactly once.
    async fn reserve(&self, request_id: &str) -> Result<(), EngineError>;

    /// Record how a reserved call ended. Idempotent for the same outcome.
    async fn record_execution(
        &self,
        request_id: &str,
        outcome: super::types::ExecutionOutcome,
    ) -> Result<(), EngineError>;

    /// Expire a pending request whose window elapsed. `Ok(true)` when this call
    /// expired it, `Ok(false)` when it was already decided (the caller then
    /// honors the decision).
    async fn expire(&self, request_id: &str) -> Result<bool, EngineError>;

    /// Forget a request the caller will not reserve or record, so the engine
    /// holds nothing for it. The request itself is left as it was decided.
    fn release(&self, request_id: &str);

    /// Cancel a pending or approved request that will not run.
    async fn cancel(&self, request_id: &str) -> Result<(), EngineError>;

    /// Whether the request is stored and has left `Pending`; `None` when it was
    /// never stored in the current scope.
    async fn is_decided(&self, request_id: &str) -> Result<Option<bool>, EngineError>;

    /// The most recently decided requests of the current scope, newest first.
    async fn history(
        &self,
        limit: usize,
    ) -> Result<Vec<super::types::ApprovalAuditEntry>, EngineError>;

    /// Durably trust one tool under the exact posture scope the request belongs
    /// to. The host calls this only after resolving the current parked request.
    async fn grant_tool_trust(
        &self,
        tenant: &Tenant,
        variant: Variant,
        tool: &str,
    ) -> Result<(), EngineError>;

    /// Read the live whole-tool grants of a registered standard approval scope.
    async fn tool_trust(
        &self,
        tenant: &Tenant,
        variant: Variant,
        posture: Posture,
    ) -> Result<Vec<super::types::ToolTrustEntry>, EngineError>;

    /// Revoke one live whole-tool grant by the opaque ID returned from
    /// [`Self::tool_trust`]. `Ok(false)` means the ID is absent in this scope.
    async fn revoke_tool_trust_by_id(
        &self,
        tenant: &Tenant,
        variant: Variant,
        posture: Posture,
        request_id: &str,
    ) -> Result<bool, EngineError>;

    /// Durably trust `tool` inside `flow_id` for the tenant: the engine keeps the
    /// grant across restarts and approves that exact tool in that exact flow
    /// without a prompt. Granting a live pair changes nothing.
    async fn grant_flow_trust(
        &self,
        tenant: &Tenant,
        flow_id: &str,
        tool: &str,
    ) -> Result<(), EngineError>;

    /// Withdraw the trust of `tool` inside `flow_id`; `Ok(false)` when there was
    /// none.
    async fn revoke_flow_trust(
        &self,
        tenant: &Tenant,
        flow_id: &str,
        tool: &str,
    ) -> Result<bool, EngineError>;

    /// The tools currently trusted inside `flow_id`, sorted.
    async fn flow_trust(&self, tenant: &Tenant, flow_id: &str) -> Result<Vec<String>, EngineError>;
}

/// The shared engine handle a gate holds.
pub(crate) type SharedEngine = Arc<dyn ApprovalEngine>;

/// What a human is shown, once the engine has made the request durable.
#[derive(Clone, Debug)]
pub(crate) struct Delivery {
    /// The call correlation the engine was given.
    pub call_id: String,
    /// The durable request identity.
    pub request_id: String,
    /// The tool, as the engine stored it.
    pub tool_name: String,
    /// The summary, as the engine stored it (redacted).
    pub summary: String,
    /// The arguments, as the engine stored them (redacted).
    pub args_redacted: Value,
    /// Creation, epoch milliseconds.
    pub created_at_ms: u64,
    /// Expiry, epoch milliseconds.
    pub expires_at_ms: u64,
}

/// The surface that shows a pending request to the user and parks its call. The
/// gate implements it for the call it is checking.
#[async_trait]
pub(crate) trait PromptSurface: Send + Sync {
    /// Show the request and route its answer. `Err` means no human can be
    /// reached, which denies the call.
    async fn deliver(&self, delivery: Delivery) -> Result<(), String>;
}

/// The engine a slim build gets: there is no module to ask, so every gated call
/// is refused rather than waved through.
#[derive(Debug, Default)]
pub(crate) struct UnavailableEngine;

impl Sealed for UnavailableEngine {}

const UNAVAILABLE: &str = "the approval engine is not part of this build (security-module is off)";

#[async_trait]
impl ApprovalEngine for UnavailableEngine {
    async fn check(&self, _input: CheckInput) -> Result<Checked, EngineError> {
        Err(EngineError::Unavailable(UNAVAILABLE.into()))
    }
    async fn decide(
        &self,
        _request_id: &str,
        _choice: Choice,
        _label: &str,
    ) -> Result<Decided, EngineError> {
        Err(EngineError::NotPending)
    }
    async fn reserve(&self, _request_id: &str) -> Result<(), EngineError> {
        Err(EngineError::Unavailable(UNAVAILABLE.into()))
    }
    async fn record_execution(
        &self,
        _request_id: &str,
        _outcome: super::types::ExecutionOutcome,
    ) -> Result<(), EngineError> {
        Err(EngineError::Unavailable(UNAVAILABLE.into()))
    }
    async fn expire(&self, _request_id: &str) -> Result<bool, EngineError> {
        Err(EngineError::Unavailable(UNAVAILABLE.into()))
    }
    fn release(&self, _request_id: &str) {}
    async fn cancel(&self, _request_id: &str) -> Result<(), EngineError> {
        Err(EngineError::Unavailable(UNAVAILABLE.into()))
    }
    async fn is_decided(&self, _request_id: &str) -> Result<Option<bool>, EngineError> {
        Ok(None)
    }
    async fn history(
        &self,
        _limit: usize,
    ) -> Result<Vec<super::types::ApprovalAuditEntry>, EngineError> {
        Ok(Vec::new())
    }
    async fn grant_tool_trust(
        &self,
        _tenant: &Tenant,
        _variant: Variant,
        _tool: &str,
    ) -> Result<(), EngineError> {
        Err(EngineError::Unavailable(UNAVAILABLE.into()))
    }
    async fn tool_trust(
        &self,
        _tenant: &Tenant,
        _variant: Variant,
        _posture: Posture,
    ) -> Result<Vec<super::types::ToolTrustEntry>, EngineError> {
        Err(EngineError::Unavailable(UNAVAILABLE.into()))
    }
    async fn revoke_tool_trust_by_id(
        &self,
        _tenant: &Tenant,
        _variant: Variant,
        _posture: Posture,
        _request_id: &str,
    ) -> Result<bool, EngineError> {
        Err(EngineError::Unavailable(UNAVAILABLE.into()))
    }
    async fn grant_flow_trust(
        &self,
        _tenant: &Tenant,
        _flow_id: &str,
        _tool: &str,
    ) -> Result<(), EngineError> {
        Err(EngineError::Unavailable(UNAVAILABLE.into()))
    }
    async fn revoke_flow_trust(
        &self,
        _tenant: &Tenant,
        _flow_id: &str,
        _tool: &str,
    ) -> Result<bool, EngineError> {
        Err(EngineError::Unavailable(UNAVAILABLE.into()))
    }
    async fn flow_trust(
        &self,
        _tenant: &Tenant,
        _flow_id: &str,
    ) -> Result<Vec<String>, EngineError> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
