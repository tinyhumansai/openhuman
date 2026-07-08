//! The "make a subconscious" surface: the [`SubconsciousKind`] enum + the single
//! [`make_subconscious`] constructor every caller (registry, trigger RPC, tests)
//! goes through. Adding a world is a profile file + one match arm here + one
//! `enabled_kinds` line — never another engine.

use serde::{Deserialize, Serialize};

use super::instance::SubconsciousInstance;
use super::profiles::{memory::memory_instance, tinyplace::tinyplace_instance};
use crate::openhuman::config::Config;

/// One instantiable subconscious world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubconsciousKind {
    /// The user's connected memory sources (Gmail / Slack / Notion / folders).
    Memory,
    /// The tiny.place orchestration steering world.
    TinyPlace,
}

impl SubconsciousKind {
    /// Every kind, in a stable order (memory first — it owns the legacy status).
    pub const ALL: [SubconsciousKind; 2] = [SubconsciousKind::Memory, SubconsciousKind::TinyPlace];

    /// Stable id — store-key namespace, log prefix, RPC name.
    pub fn id(self) -> &'static str {
        match self {
            SubconsciousKind::Memory => "memory",
            SubconsciousKind::TinyPlace => "tinyplace",
        }
    }

    /// Parse a kind id (`"memory"` | `"tinyplace"`); `None` on anything else.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "memory" => Some(SubconsciousKind::Memory),
            "tinyplace" => Some(SubconsciousKind::TinyPlace),
            _ => None,
        }
    }

    /// Which kinds should run for this config — the bootstrap set.
    ///
    /// - `Memory`    ⇐ `heartbeat.enabled && mode != Off` (the pre-factory gate).
    /// - `TinyPlace` ⇐ `orchestration.enabled` (the pre-factory review gate).
    pub fn enabled_kinds(config: &Config) -> Vec<Self> {
        let mut kinds = Vec::new();
        if config.heartbeat.enabled && config.heartbeat.effective_subconscious_mode().is_enabled() {
            kinds.push(SubconsciousKind::Memory);
        }
        if config.orchestration.enabled {
            kinds.push(SubconsciousKind::TinyPlace);
        }
        kinds
    }
}

/// The only place profiles are constructed into a runner.
pub fn make_subconscious(kind: SubconsciousKind, config: &Config) -> SubconsciousInstance {
    match kind {
        SubconsciousKind::Memory => memory_instance(config),
        SubconsciousKind::TinyPlace => tinyplace_instance(config),
    }
}

#[cfg(test)]
#[path = "factory_tests.rs"]
mod tests;
