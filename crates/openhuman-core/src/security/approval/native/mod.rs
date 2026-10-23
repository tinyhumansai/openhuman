//! The native approval engine: the TinySecurity module behind its bus contract.
//!
//! [`NativeEngine`] implements the gate's [`ApprovalEngine`](super::engine) by
//! registering each subject with the module, allocating the audit correlation of
//! each phase, and calling `EvaluateApproval2`, `Decide2`, `ConsumeApproval2`,
//! `RecordExecution2`, `ExpireApproval2` and `CancelApproval2`. The module
//! calls back into the host through [`callbacks`], which serve its store, its
//! audit sink, the prompt and the human verification from this directory's
//! state.
//!
//! Layout:
//!
//! | File | Role |
//! | --- | --- |
//! | `engine.rs` | `NativeEngine`, the module calls |
//! | `scopes.rs` | subject registration, posture and grants |
//! | `audit_stream.rs` | a subject's audit stream, leased by the sandbox planner |
//! | `rows.rs` | the document-port store behind `Load`, `Cas` and `Append` |
//! | `history_import.rs` | one-time import of the legacy `approval.db` into the module |
//! | `callbacks.rs` | what `modules/security_host.rs` serves |
//! | `state.rs` | calls, requests and human tickets in flight |
//! | `view.rs` | history read from the stored rows |

use sha2::{Digest, Sha256};

pub(crate) mod audit_stream;
pub(crate) mod callbacks;
mod engine;
#[cfg(feature = "legacy-import")]
mod history_import;
mod rows;
mod scopes;
mod state;
mod view;

pub(crate) use engine::NativeEngine;

/// The SHA-256 correlation of a ticket the module stores instead of the ticket.
pub(super) fn ticket_sha256(ticket: &str) -> String {
    format!("{:x}", Sha256::digest(ticket.as_bytes()))
}
