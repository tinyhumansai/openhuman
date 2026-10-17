//! The [`AgentProgress`] channel type an **in-process embedder** hands a turn it
//! drives through an RPC that only returns a final string.
//!
//! The agent harness already emits everything an embedder needs
//! ([`AgentProgress`], attached with
//! [`OpenHumanSessionHost::set_on_progress`](crate::agent::OpenHumanSessionHost::set_on_progress)),
//! but entry points such as
//! [`agent_chat_for`](crate::inference::host_runtime::ops::agent_chat_for) build
//! the [`Agent`](crate::agent::OpenHumanSessionHost) internally and drop it, so
//! there is no handle to attach a sink to. They therefore take the sink as an
//! explicit argument (`ops::TurnSinks`) and attach it to the session they build:
//! the sink reaches exactly the turn it was handed to and no other, including
//! across a `tokio::spawn`, which an ambient scope would silently lose.
//!
//! An entry point must not clobber a sink an existing caller already attached
//! explicitly (web chat, the platform socket, flows, skills all call
//! `set_on_progress` themselves): where both could apply, the **explicitly-set
//! sink wins**.

use crate::agent::progress::AgentProgress;

/// The channel an embedder hands to a turn, and the same type
/// [`OpenHumanSessionHost::set_on_progress`](crate::agent::OpenHumanSessionHost::set_on_progress)
/// takes — so it can be attached directly.
///
/// Use a bounded channel with headroom: the turn loop sends from inside the
/// agent, so a slow or absent receiver applies backpressure to the turn itself.
pub type ProgressSink = tokio::sync::mpsc::Sender<AgentProgress>;
