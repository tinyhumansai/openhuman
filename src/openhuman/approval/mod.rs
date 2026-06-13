//! Interactive approval workflow for supervised mode.
//!
//! [`ApprovalGate`] (in [`gate`]) is the single approval path — async
//! middleware between the agent and any tool whose
//! [`crate::openhuman::tools::Tool::external_effect`] returns `true`. It
//! persists pending rows in SQLite, parks the tool-call future on a oneshot,
//! and resumes when the UI dispatches `approval_decide`. Introduced for issue
//! #1339 so external-channel writes (Slack post, email send, calendar create,
//! …) cannot fire without explicit user consent.
//!
//! The user's "Always allow" allowlist lives in `autonomy.auto_approve` (read
//! by the gate via [`crate::openhuman::security::SecurityPolicy`]); a prior
//! list-based `ApprovalManager` that consumed it was removed once the gate
//! became the sole control.

pub mod gate;
pub mod redact;
pub mod rpc;
pub mod schemas;
pub mod store;
pub mod types;

pub use gate::{parse_approval_reply, ApprovalChatContext, ApprovalGate, APPROVAL_CHAT_CONTEXT};
pub use redact::{redact_args, summarize_action};
pub use schemas::all_controller_schemas as all_approval_controller_schemas;
pub use schemas::all_registered_controllers as all_approval_registered_controllers;
pub use types::{
    ApprovalAuditEntry, ApprovalDecision, ExecutionOutcome, GateOutcome, PendingApproval,
};
