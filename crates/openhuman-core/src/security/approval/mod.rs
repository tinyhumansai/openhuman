//! Interactive approval workflow for supervised mode.
//!
//! [`ApprovalGate`] (in [`gate`]) is the single approval path — async
//! middleware between the agent and any tool whose
//! [`tinytools::Tool::external_effect`] returns `true`. It asks the approval
//! engine ([`engine`]) whether the call may run, parks the tool-call future on
//! a oneshot when a person must decide, and resumes when the UI dispatches
//! `approval_decide`. Introduced for issue #1339 so external-channel writes
//! (Slack post, email send, calendar create, …) cannot fire without explicit
//! user consent.
//!
//! The engine is the TinySecurity module ([`native`], behind
//! `security-module`): it owns each request's durable lifecycle, its exact
//! binding to the call, the 10-minute expiry and the audit trail, and the host
//! serves the module's store, audit sink, prompt and human verification. The
//! Whole-tool grants live in TinySecurity's durable approval state and are bound
//! to the exact user/workspace/agent approval scope. The legacy
//! `autonomy.auto_approve` list is retained for config compatibility but is not
//! replayed into the module, so revoking a durable grant remains effective.
//! Per-flow trust is also module-owned (`GrantFlowTool2`).

pub(crate) mod engine;
#[cfg(test)]
pub(crate) mod engine_stub;
mod flow_surface;
pub mod gate;
#[cfg(feature = "security-module")]
pub(crate) mod native;
mod registration_scope;
pub mod summary;
pub use registration_scope::ApprovalScope;
pub mod rpc;
pub mod schemas;
mod storage_schema;
pub mod store;
mod store_documents;
#[cfg(all(feature = "security-module", feature = "legacy-import"))]
mod store_import;
pub(crate) use storage_schema::{collection_specs, upgrade_records};
pub mod types;

pub use gate::{
    is_detached_request, is_unanswered_approval_reason, parse_approval_reply, ApprovalChatContext,
    ApprovalError, ApprovalGate, ApprovalTurnContext, FlowRunContext, APPROVAL_UNANSWERED_PHRASE,
};
pub use schemas::all_controller_schemas as all_approval_controller_schemas;
pub use schemas::all_registered_controllers as all_approval_registered_controllers;
#[cfg(test)]
pub(crate) use schemas::approval_parameter_contracts;
pub use summary::summarize_action;
pub use types::{
    ApprovalAuditEntry, ApprovalDecision, ApprovalSourceContext, ExecutionOutcome, GateOutcome,
    JudgeVerdict, PendingApproval, ToolTrustEntry,
};
