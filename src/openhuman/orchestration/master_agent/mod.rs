//! The `master_agent` built-in: **OpenHuman talking directly to its human** in the
//! Master chat (human ↔ OpenHuman). Same deep-thinking tier + tiny.place tool
//! belt as the [`super::reasoning_agent`], but a human-facing system prompt — NO
//! A2A "split-brain / you are not talking to the user" framing.
//!
//! The wake graph's `execute` node runs this agent (instead of the reasoning
//! core) for a **local** Master cycle — counterpart = [`super::super::types::LOCAL_MASTER_AGENT`]
//! (see [`super::ops`]). Peer-initiated / A2A cycles keep using `reasoning_agent`.
//!
//! Registered in the built-in loader
//! ([`crate::openhuman::agent_registry::agents::loader`]); reuses the reasoning
//! core's per-cycle steering task-local for the steering directive.

pub mod prompt;
