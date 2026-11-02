//! Real-time progress events emitted during an agent turn.
//!
//! The event vocabulary is owned by TinyAgents
//! (`tinyagents_harness::progress`); this module re-exports it so host code
//! can keep naming `crate::progress::AgentProgress` while every producer and
//! consumer (session host, sub-agent host, web chat, channels, voice) shares
//! one type.
//!
//! Consumers (e.g. the web channel provider) create an
//! `mpsc::Sender<AgentProgress>` and attach it to the session host via
//! `OpenHumanSessionHost::set_on_progress` before running a turn. The turn
//! loop sends events through this channel as it progresses: tool calls
//! starting/completing, iteration boundaries, sub-agent lifecycle, etc.
//!
//! This is intentionally separate from `DomainEvent` (the global broadcast
//! bus) because progress events are **per-request scoped**: they carry no
//! routing info (client_id, thread_id); the consumer that created the channel
//! already knows those and tags the outgoing socket events accordingly.

pub use tinyagents_harness::progress::{AgentProgress, SubagentUsage};
