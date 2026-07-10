//! Tool-call lifecycle state and human-readable failure classification.
//!
//! Foundation for the "Visible tool status, failure diagnosis, and safe
//! recovery flows" epic (#4254). This module owns the shared vocabulary that
//! later phases (status panel, bounded retry, in-app diagnostics) build on:
//!
//! - [`ToolLifecycleState`] — where a call is (queued/running/…/needs-input).
//! - [`ToolFailureClass`] / [`FailureCategory`] — what kind of failure it was.
//! - [`ClassifiedFailure`] — a class plus plain-language cause + next action.
//! - [`classify`] — the pure heuristic mapping raw tool error text → the above.
//!
//! It owns no agent tools, no persistence, and no event subscribers; it is a
//! pure data + logic module consumed by the agent tool executor and surfaced
//! over the `tool` event-bus domain.

mod ops;
mod types;

pub use ops::{classify, describe};
pub use types::{ClassifiedFailure, FailureCategory, ToolFailureClass, ToolLifecycleState};
