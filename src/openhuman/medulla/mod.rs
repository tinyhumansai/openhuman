//! Medulla integration domain — the cloud client, its wire vocabulary, and the
//! harness contract types the two share.
//!
//! This is the real Medulla surface. It replaced the `medulla_local` draft — a
//! supervised `medulla-serve` child that answered two of N ports — which has
//! since been removed along with the `subconscious.engine = "medulla"`
//! behaviour it backed. That engine is to be re-ported onto this domain; its
//! config keys remain accepted as inert serde meanwhile so existing configs
//! still boot (see `config::schema::subconscious`).
//!
//! # Two directions, one product name
//!
//! Do not confuse this domain with `socket::medulla`. They are unrelated code
//! paths that share nothing but a word:
//!
//! | Module | Role | Transport |
//! |---|---|---|
//! | `openhuman::medulla` (here) | OpenHuman as a Medulla **client** | outbound HTTP/SSE to the backend |
//! | `openhuman::platform::socket::medulla` | OpenHuman as a Medulla **worker** | inbound Socket.IO from a remote operator |
//!
//! A single binary can be both at once.
//!
//! # Gating
//!
//! Behaviour is gated on the `medulla` Cargo feature and tagged
//! [`DomainGroup::Medulla`](crate::core::all::DomainGroup) at runtime. The
//! [`contract`] and [`events`] type modules are **ungated carve-outs** — see
//! [`events`] for why.

/// Medulla chat-session store (`medulla_chat`). Gated on the same `medulla`
/// feature as the rest of the family.
#[cfg(feature = "medulla")]
pub mod chat;
#[cfg(feature = "medulla")]
pub mod client;
pub mod contract;
pub mod events;
#[cfg(feature = "medulla")]
pub mod ops;
#[cfg(feature = "medulla")]
pub mod resolve;
#[cfg(feature = "medulla")]
mod schemas;

pub use contract::{VerificationEvidence, WorkerContract};
pub use events::{EventEnvelope, SessionEvent};
#[cfg(feature = "medulla")]
pub use schemas::{all_medulla_controller_schemas, all_medulla_registered_controllers};

/// The tool names the Medulla harness reserves for its built-in memory and
/// task-tracker modules.
///
/// A module author registering business tools must avoid these: the harness
/// composes its own modules eagerly and a collision throws at construction.
pub const RESERVED_TOOL_NAMES: [&str; 6] = [
    "task_create",
    "task_update",
    "task_list",
    "memory_write",
    "memory_read",
    "memory_list",
];

/// Whether `name` collides with a harness-reserved tool name.
pub fn is_reserved_tool_name(name: &str) -> bool {
    RESERVED_TOOL_NAMES.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_tool_names_are_detected() {
        for name in RESERVED_TOOL_NAMES {
            assert!(is_reserved_tool_name(name), "{name} must be reserved");
        }
    }

    #[test]
    fn ordinary_tool_names_are_not_reserved() {
        for name in ["file_read", "web_fetch", "task_creates", "memory"] {
            assert!(!is_reserved_tool_name(name), "{name} must not be reserved");
        }
    }
}
