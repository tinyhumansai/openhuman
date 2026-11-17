//! Translate [`AgentProgress`] events into [`TurnState`] mutations and
//! flush snapshots to disk at iteration / tool boundaries.
//!
//! Used by the web-channel progress bridge to keep an authoritative,
//! restart-survivable record of the in-flight turn alongside the live
//! socket emissions. High-frequency deltas (text, thinking, tool args)
//! mutate the in-memory snapshot but do not trigger a disk flush —
//! anything more granular than an iteration / tool boundary would
//! thrash the filesystem under streaming load.
//!
//! On [`AgentProgress::TurnCompleted`] the snapshot is marked
//! [`TurnLifecycle::Completed`] and kept on disk so a reloaded client can
//! replay the finished turn. If the bridge exits without ever observing
//! `TurnCompleted` (for example because the agent loop returned an error),
//! the snapshot is flagged [`TurnLifecycle::Interrupted`] and persisted so
//! the UI can surface a retry affordance. The mirror type and that
//! finalization live in `tinyagents_session::turn_state::TurnStateMirror`.

#[cfg(test)]
#[path = "mirror_tests.rs"]
mod tests;

mod observe;

pub use observe::ObserveProgress;

#[cfg(test)]
use tinyagents_session::port::{InMemoryTurnStates, TurnStates};
#[cfg(test)]
use tinyagents_session::turn_state::mirror::MAX_PERSISTED_TRANSCRIPT_ITEM;
#[cfg(test)]
use tinyagents_session::turn_state::types::{
    SubagentToolCall, SubagentTranscriptItem, ToolTimelineStatus, TranscriptItem, TurnLifecycle,
    TurnPhase,
};
#[cfg(test)]
use tinyagents_session::turn_state::TurnStateMirror;
