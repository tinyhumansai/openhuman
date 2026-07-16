//! Skill registry: browse, search, and install skills from the aggregated
//! Hermes catalog (HermesHub, ClawHub, skills.sh, LobeHub, browse.sh)
//! with local caching.
//!
//! ## Compile-time gate (`skills` feature)
//!
//! `pub mod skill_registry;` is ALWAYS compiled — it is a facade. The real
//! implementation is gated behind the default-ON `skills` Cargo feature (the
//! same gate as `openhuman::skills` and `openhuman::skill_runtime` — the three
//! domains ship as one unit). When the feature is off, [`stub`] takes its
//! place with no-op / empty bodies. See `src/openhuman/skills/mod.rs` for the
//! pattern and the type carve-out.

#[cfg(feature = "skills")]
pub mod agent;
#[cfg(feature = "skills")]
pub mod ops;
#[cfg(feature = "skills")]
pub mod schemas;
#[cfg(feature = "skills")]
pub mod store;
#[cfg(feature = "skills")]
pub mod tools;
#[cfg(feature = "skills")]
pub mod types;

#[cfg(feature = "skills")]
pub use schemas::{
    all_skill_registry_controller_schemas, all_skill_registry_registered_controllers,
};

/// Serializes tests that mutate the process-global `OPENHUMAN_SKILL_REGISTRY_CACHE_DIR`
/// env var, so cargo's parallel runner can't interleave their cache dirs.
#[cfg(all(test, feature = "skills"))]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

// ---------------------------------------------------------------------------
// Disabled facade — compiled only when the `skills` feature is OFF.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "skills"))]
mod stub;
#[cfg(not(feature = "skills"))]
pub use stub::*;
