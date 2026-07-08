//! The `master_reporter` built-in: a **tool-free** relay that reports an external
//! agent's reply back into the Master chat as OpenHuman's own message.
//!
//! A peer reply is untrusted input. `report_peer_reply_to_master`
//! ([`super::ops`]) runs THIS agent — not [`super::master_agent`] — so the peer
//! text never reaches OpenHuman's tiny.place tool belt or sub-agents and cannot
//! prompt-inject OpenHuman into reading sessions or messaging contacts.
//!
//! Registered in the built-in loader
//! ([`crate::openhuman::agent_registry::agents::loader`]).

pub mod prompt;
