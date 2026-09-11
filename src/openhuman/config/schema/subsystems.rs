//! `[subsystems.*]` config section — the uniform cross-subsystem driver-binding
//! shape defined in `docs/specs/kernel.md` §3.6 and `docs/specs/plan-memory.md` §4.5.
//!
//! The definitions moved to [`tinymemory_api::host`]: `tinymemory-core`'s
//! driver binding reads `MemorySubsystemConfig` field by field, so the struct
//! had to travel with it. Their serde form is persisted in users' `config.toml`
//! and is a compatibility surface.
//!
//! Re-exported here so every existing `config::schema::subsystems::…` path keeps
//! resolving. The round-trip test below stays on this side of the seam because
//! it parses a whole [`Config`](super::Config), which the contract crate cannot
//! name.

pub use tinymemory_api::host::subsystems::{
    MemoryDriverConfig, MemoryHooksConfig, MemorySubsystemConfig, SubsystemsConfig,
};

#[cfg(test)]
#[path = "subsystems_tests.rs"]
mod tests;
