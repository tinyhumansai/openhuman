//! Skill registry: browse, search, and install skills from the aggregated
//! Hermes catalog (HermesHub, ClawHub, skills.sh, LobeHub, browse.sh)
//! with local caching.

pub mod agent;
pub mod ops;
pub mod schemas;
pub mod store;
pub mod tools;
pub mod types;

pub use schemas::{
    all_skill_registry_controller_schemas, all_skill_registry_registered_controllers,
};

/// Serializes tests that mutate the process-global `OPENHUMAN_SKILL_REGISTRY_CACHE_DIR`
/// env var, so cargo's parallel runner can't interleave their cache dirs.
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
