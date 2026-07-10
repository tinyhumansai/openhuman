//! The "make a subconscious" surface: the [`SubconsciousKind`] enum + the single
//! [`make_subconscious`] constructor every caller (registry, trigger RPC, tests)
//! goes through. Adding a world is a profile file + one match arm here + one
//! `enabled_kinds` line — never another engine.

use serde::{Deserialize, Serialize};

use super::instance::SubconsciousInstance;
use super::profiles::memory::memory_instance;
use crate::openhuman::config::Config;

/// One instantiable subconscious world.
///
/// The tiny.place orchestration-steering world was retired when the orchestration
/// brain moved server-side — the hosted subconscious tier now runs that review,
/// triggered by the device's world-diff uploads. Only the `Memory` world runs on
/// the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubconsciousKind {
    /// The user's connected memory sources (Gmail / Slack / Notion / folders).
    Memory,
}

impl SubconsciousKind {
    /// Every kind, in a stable order (memory first — it owns the legacy status).
    pub const ALL: [SubconsciousKind; 1] = [SubconsciousKind::Memory];

    /// Stable id — store-key namespace, log prefix, RPC name.
    pub fn id(self) -> &'static str {
        match self {
            SubconsciousKind::Memory => "memory",
        }
    }

    /// Parse a kind id (`"memory"`); `None` on anything else.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "memory" => Some(SubconsciousKind::Memory),
            _ => None,
        }
    }

    /// Which kinds should run for this config — the bootstrap set.
    ///
    /// - `Memory` ⇐ `heartbeat.enabled && mode != Off` (the pre-factory gate).
    pub fn enabled_kinds(config: &Config) -> Vec<Self> {
        let mut kinds = Vec::new();
        if config.heartbeat.enabled && config.heartbeat.effective_subconscious_mode().is_enabled() {
            kinds.push(SubconsciousKind::Memory);
        }
        kinds
    }
}

/// The only place profiles are constructed into a runner.
pub fn make_subconscious(kind: SubconsciousKind, config: &Config) -> SubconsciousInstance {
    match kind {
        SubconsciousKind::Memory => memory_instance(config),
    }
}

#[cfg(test)]
#[path = "factory_tests.rs"]
mod tests;
