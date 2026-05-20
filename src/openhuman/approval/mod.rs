//! Interactive approval workflow for supervised mode.
//!
//! Two layers:
//!
//! - [`ApprovalManager`] (legacy, in [`ops`]) — CLI-only synchronous
//!   prompt + in-memory session allowlist + audit log. Still used by
//!   the agent harness when running under `--channel cli`.
//! - [`ApprovalGate`] (new, in [`gate`]) — async middleware between the
//!   agent and any tool whose [`crate::openhuman::tools::Tool::external_effect`]
//!   returns `true`. Persists pending rows in SQLite, parks the
//!   tool-call future on a oneshot, and resumes when the UI dispatches
//!   `approval_decide`. Introduced for issue #1339 so external-channel
//!   writes (Slack post, email send, calendar create, …) cannot fire
//!   without explicit user consent.

pub mod gate;
pub mod ops;
pub mod redact;
pub mod rpc;
pub mod schemas;
pub mod store;
pub mod types;

pub use gate::ApprovalGate;
pub use ops::*;
pub use redact::{redact_args, summarize_action};
pub use schemas::all_controller_schemas as all_approval_controller_schemas;
pub use schemas::all_registered_controllers as all_approval_registered_controllers;
pub use types::{ApprovalDecision, GateOutcome, PendingApproval};
