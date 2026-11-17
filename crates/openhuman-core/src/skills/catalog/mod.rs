//! Skill registry: browse, search, and install skills from the aggregated
//! Hermes catalog (HermesHub, ClawHub, skills.sh, LobeHub, browse.sh)
//! through the tinyskills `SkillRegistry`.
//!
//! ## Compile-time gate (`skills` feature)
//!
//! `pub mod catalog;` is ALWAYS compiled — it is a facade. The real
//! implementation is gated behind the default-ON `skills` Cargo feature (the
//! same gate as `openhuman::skills` and `openhuman::skills::runtime` — the three
//! domains ship as one unit). When the feature is off, [`stub`] takes its
//! place with no-op / empty bodies. See `crates/openhuman-core/src/skills/mod.rs` for the
//! pattern and the type carve-out.

#[cfg(feature = "skills")]
pub mod agent;
#[cfg(feature = "skills")]
pub mod ops;
#[cfg(feature = "skills")]
mod registry;
#[cfg(feature = "skills")]
pub mod schemas;
#[cfg(feature = "skills")]
mod store;
#[cfg(all(test, feature = "skills"))]
pub(crate) mod test_fixtures;
#[cfg(feature = "skills")]
pub mod tools;
#[cfg(feature = "skills")]
mod transport;
#[cfg(feature = "skills")]
pub mod types;

#[cfg(feature = "skills")]
pub(crate) use registry::registry_timeouts;
#[cfg(feature = "skills")]
pub use registry::{skill_registry, HERMES_REGISTRY_ID, REFRESH_COOLDOWN};
#[cfg(feature = "skills")]
pub use transport::ReqwestTransport;

#[cfg(feature = "skills")]
pub use schemas::{
    all_skill_registry_controller_schemas, all_skill_registry_registered_controllers,
};

/// Serializes tests that mutate the registry's process-global environment or
/// its shared handle.
#[cfg(all(test, feature = "skills"))]
pub(crate) static TEST_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ---------------------------------------------------------------------------
// Disabled facade — compiled only when the `skills` feature is OFF.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "skills"))]
mod stub;
#[cfg(not(feature = "skills"))]
pub use stub::*;
