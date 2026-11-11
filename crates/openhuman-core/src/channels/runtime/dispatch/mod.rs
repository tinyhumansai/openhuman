//! Channel runtime loop and message processing.
//!
//! Sub-modules:
//! * [`helpers`]   — small stateless helpers (context block, ACK reaction, typing, workers).
//! * [`host_agent`] — channels bound to a host-registered agent
//!   (`agent.channel_agents`): routing, the refusal for a missing agent, the
//!   turn runner and the external-channel posture.
//! * [`routing`]   — agent selection and tool-scoping ([`AgentScoping`],
//!   [`resolve_target_agent`], [`build_visible_tool_set`]).
//! * [`processor`] — core message pipeline ([`process_channel_message`],
//!   [`run_message_dispatch_loop`]) and approval-surface gate.

mod helpers;
pub(crate) mod host_agent;
mod inbound_batch;
mod native_runner;
mod processor;

#[cfg(test)]
pub(crate) use processor::drop_media_read_lease_for_test;
mod routing;

pub(crate) use processor::progressive::{
    delivery_succeeded, hosted_relay_target, NativeProgressiveTarget,
};

#[cfg(test)]
pub(crate) use processor::process_channel_message;
pub(crate) use processor::{
    process_channel_runtime_message, reset_durable_channel_session, run_message_dispatch_loop,
    RuntimeChannelMessage,
};

// `channel_has_approval_surface` stays pub(crate) on processor; re-export so
// the inline test module can reach it via `super::channel_has_approval_surface`.
#[cfg(test)]
use processor::channel_has_approval_surface;

// Re-export internal helpers accessed by test_support (cfg(any(test,
// debug_assertions))) and the inline #[cfg(test)] modules via `super::*`.
#[cfg(any(test, debug_assertions))]
pub(crate) use helpers::build_channel_context_block;
#[cfg(test)]
use helpers::select_acknowledgment_reaction;

#[cfg(test)]
use routing::{build_visible_tool_set, AgentScoping};

#[cfg(any(test, debug_assertions))]
use tinychannels_bus as traits;

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
