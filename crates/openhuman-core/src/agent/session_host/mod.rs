//! Stateful agent session — the single execution tier.
//!
//! This module owns the [`OpenHumanSessionHost`] struct, which drives per-turn
//! interaction with the provider, tool registry, memory system, and
//! hook pipeline. It is the runtime the `channels`, `local_ai`, and
//! `cron` layers invoke when they need a conversation to make
//! progress.
//!
//! # Product shell over TinyAgents
//!
//! Every production turn is a `tinyagents_runtime::Session` transition. The
//! OpenHuman driver supplies model/tool execution; codec and hooks supply the
//! product transcript dialect, prompt policy, and post-durability effects.
//! The runtime, rather than this host, owns generic history, raw transcript
//! reconciliation, prefix stability, tool snapshots, resume, and persistence.
//!
//! # File layout
//!
//! | File          | Role                                                             |
//! |---------------|------------------------------------------------------------------|
//! | [`types`]     | `OpenHumanSessionHost` and `SessionHostBuilder` struct definitions (no logic).        |
//! | [`builder`]   | `SessionHostBuilder` fluent API + `OpenHumanSessionHost::from_config` factory.        |
//! | [`runtime_session`] | Runtime session composition and public `turn()`.          |
//! | [`driver`]    | OpenHuman model/harness `SessionDriver`.                         |
//! | [`hooks`]     | Product preparation/commit/terminal hooks.                       |
//! | [`runtime`]   | Public accessors and `run_single` / `run_interactive`.           |
//! | [`session_key`] | Minting and renaming the per-host transcript key.              |
//!
//! External callers should import [`OpenHumanSessionHost`] and [`SessionHostBuilder`] from
//! `crate::agent`, which re-exports them from this module.
//! The child files are an implementation detail.

#[cfg(feature = "flows")]
pub(crate) use builder::provider_role_for_definition;
pub use builder::{HostTools, HostTurnTools, TurnContext};

mod builder;
mod builder_config;
mod codec;
mod driver;
pub(crate) mod product_copy;
// Tool surface for live voice sessions (`voice::live`).
#[cfg(feature = "voice")]
mod live_tools;
mod runtime;
mod runtime_session;
mod session_key;
pub(crate) mod stop_policy;
mod types;

#[cfg(test)]
#[path = "test_support.rs"]
pub(crate) mod test_support;

pub use codec::OpenHumanTranscriptCodec;
pub type OpenHumanSessionHooks =
    tinyagents_runtime::FnSessionHooks<crate::agent::tinyagents::host::OpenHumanRunContext>;
pub use runtime_session::{CompactOptions, CompactOutcome};
pub use types::{OpenHumanSessionHost, SessionHostBuilder, TurnOverrides};

// Re-export the duplicate-tool-spec guard for sibling harness modules
// (`session::runtime`, `subagent_host`) so all provider call sites
// share one tested implementation.

#[cfg(test)]
#[path = "imported_history_tests.rs"]
mod imported_history_tests;
#[cfg(test)]
mod orphaned_head_resume_tests;
#[cfg(test)]
mod response_shape_turn_tests;
#[cfg(test)]
mod runtime_adapter_session_state_tests;
#[cfg(test)]
mod runtime_adapter_tests;
#[cfg(test)]
mod runtime_adapter_trace_tests;
#[cfg(test)]
mod runtime_adapter_usage_tests;
#[cfg(all(test, feature = "legacy-import"))]
mod transcript_compat_tests;
#[cfg(all(test, feature = "legacy-import"))]
#[path = "transcript_resume_stable_tests.rs"]
mod transcript_resume_stable_tests;
#[cfg(all(test, feature = "legacy-import"))]
mod typed_transcript_compat_tests;
