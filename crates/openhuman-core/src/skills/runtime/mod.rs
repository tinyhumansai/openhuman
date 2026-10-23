//! Skill runtime: execution, cancellation, and run-log polling for installed
//! SKILL.md workflows.
//!
//! `crate::skills` owns discovery and installed skill metadata.
//! `crate::skills::catalog` owns remote catalogs and install sources. This
//! module owns actually running a skill, regardless of whether the skill's
//! instructions call Python, Node, shell tools, or another OpenHuman agent tool.
//!
//! ## Compile-time gate (`skills` feature)
//!
//! `pub mod runtime;` is ALWAYS compiled — it is a facade. The real
//! implementation is gated behind the default-ON `skills` Cargo feature (the
//! same gate as `openhuman::skills` and `openhuman::skills::catalog` — the
//! three domains ship as one unit). When the feature is off, [`stub`] takes
//! its place with disabled-error / empty bodies. See
//! `crates/openhuman-core/src/skills/mod.rs` for the pattern and the type carve-out.

#[cfg(feature = "skills")]
pub mod ops;
#[cfg(feature = "skills")]
mod run_machinery;
#[cfg(feature = "skills")]
pub mod schemas;
#[cfg(feature = "skills")]
pub mod tools;

#[cfg(feature = "skills")]
pub use run_machinery::{await_run_outcome, spawn_workflow_run_background, WorkflowRunStarted};
#[cfg(feature = "skills")]
pub use schemas::{
    all_skill_runtime_controller_schemas, all_skill_runtime_registered_controllers,
    skill_runtime_schemas,
};

// ---------------------------------------------------------------------------
// Disabled facade — compiled only when the `skills` feature is OFF.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "skills"))]
mod stub;
#[cfg(not(feature = "skills"))]
pub use stub::*;
#[cfg(feature = "skills")]
pub(crate) mod skill_scan;

#[cfg(all(feature = "skills", test))]
#[path = "skill_scan_tests.rs"]
mod skill_scan_tests;
