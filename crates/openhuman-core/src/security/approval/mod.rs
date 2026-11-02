//! Interactive approval workflow for supervised mode.
//!
//! [`ApprovalGate`] (in [`gate`]) is the single approval path — async
//! middleware between the agent and any tool whose
//! [`tinytools::Tool::external_effect`] returns `true`. It
//! persists pending rows in the storage backend, parks the tool-call future on a oneshot,
//! and resumes when the UI dispatches `approval_decide`. Introduced for issue
//! #1339 so external-channel writes (Slack post, email send, calendar create,
//! …) cannot fire without explicit user consent.
//!
//! The user's "Always allow" allowlist lives in `autonomy.auto_approve` (read
//! by the gate via [`crate::security::SecurityPolicy`]); a prior
//! list-based `ApprovalManager` that consumed it was removed once the gate
//! became the sole control.

pub(crate) mod engine;
#[cfg(test)]
pub(crate) mod engine_stub;
mod flow_surface;
pub mod gate;
#[cfg(feature = "security-module")]
pub(crate) mod native;
mod registration_scope;
mod summary;
pub use registration_scope::ApprovalScope;
pub mod rpc;
pub mod schemas;
mod storage_schema;
mod store_documents;
#[cfg(feature = "legacy-import")]
mod store_import;
pub(crate) use storage_schema::{collection_specs, upgrade_records};
#[cfg(feature = "legacy-import")]
pub(crate) use store_documents::import_source;
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
    PendingApproval,
};
