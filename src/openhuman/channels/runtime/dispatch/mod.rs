//! Channel runtime loop and message processing.
//!
//! Sub-modules:
//! * [`helpers`]   — small stateless helpers (context block, ACK reaction, typing, workers).
//! * [`routing`]   — agent selection and tool-scoping ([`AgentScoping`],
//!   [`resolve_target_agent`], [`build_visible_tool_set`]).
//! * [`processor`] — core message pipeline ([`process_channel_message`],
//!   [`run_message_dispatch_loop`]) and approval-surface gate.

mod helpers;
mod processor;
mod routing;

pub(crate) use processor::{
    process_channel_message, process_channel_runtime_message, run_message_dispatch_loop,
    RuntimeChannelMessage,
};

// `channel_has_approval_surface` stays pub(crate) on processor; re-export so
// the inline test module can reach it via `super::channel_has_approval_surface`.
#[cfg(test)]
use processor::channel_has_approval_surface;

// Re-export internal helpers accessed by test_support (cfg(any(test,
// debug_assertions))) and the inline #[cfg(test)] modules via `super::*`.
#[cfg(any(test, debug_assertions))]
use helpers::{build_channel_context_block, select_acknowledgment_reaction};

#[cfg(test)]
use helpers::{contains_any, starts_with_any};

#[cfg(test)]
use routing::{build_visible_tool_set, AgentScoping};

#[cfg(any(test, debug_assertions))]
use crate::openhuman::channels::traits;

#[cfg(any(test, debug_assertions))]
#[path = "mod_test_support_tests.rs"]
pub mod test_support;

#[cfg(test)]
#[path = "mod_scoping_tests_tests.rs"]
mod scoping_tests;

#[cfg(test)]
#[path = "mod_approval_surface_gating_tests_tests.rs"]
mod approval_surface_gating_tests;

#[cfg(test)]
#[path = "../dispatch_tests.rs"]
mod tests;
