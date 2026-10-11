//! Skills metadata helpers (discovery, parse, install, run).
//!
//! ## Compile-time gate (`skills` feature)
//!
//! `pub mod skills;` is ALWAYS compiled — it is a facade. The behavioural
//! submodules below are gated behind the default-ON `skills` Cargo feature;
//! when it is off, [`stub`] takes their place and exposes the same public
//! surface that always-on callers depend on (`load_workflow_metadata`,
//! `init_workflows_dir`, `registry`, `bus`, the controller aggregators) with
//! no-op / empty bodies. Callers therefore do **not** need per-call `#[cfg]`.
//! (The `tools` glob is `#[cfg(feature = "skills")]` at its re-export site in
//! `tools/mod.rs`, so the stub omits it — mirroring the `voice` gate.)
//!
//! Workflow metadata comes directly from `tinyskills` in both feature directions.
//! [`ops_types`] keeps OpenHuman sidecar names and its trust marker. Generic tool
//! result types come directly from `tinytools`.
//!
//! The disabled facade mirrors behavior functions and preserves those metadata
//! types. Check both feature directions when changing its signatures.

// Type carve-out: always compiled, both feature directions. See module docs.
pub mod ops_types;

// Facade children — the `pub mod` line stays UNGATED because each child is
// itself a facade+stub for the same `skills` feature and its stub must resolve
// in a skills-less build (the `meet/` pilot's rule). `webhooks` is ungated
// outright: it has always-compiled callers in `crates/openhuman-core/src/core/` and is not part of
// the `skills` gate at all.
pub mod catalog;
pub mod runtime;
pub mod webhooks;

#[cfg(feature = "skills")]
pub mod bundled;
#[cfg(feature = "skills")]
pub mod bus;
#[cfg(feature = "skills")]
pub mod ops;
#[cfg(feature = "skills")]
pub mod ops_create;
#[cfg(feature = "skills")]
pub mod ops_discover;
#[cfg(feature = "skills")]
pub mod ops_install;
#[cfg(feature = "skills")]
pub mod preflight;
#[cfg(feature = "skills")]
pub(crate) mod registration;
#[cfg(feature = "skills")]
pub mod registry;
#[cfg(feature = "skills")]
pub mod run_log;
#[cfg(feature = "skills")]
pub mod schemas;
#[cfg(feature = "skills")]
pub mod search;
#[cfg(feature = "skills")]
pub mod tools;
#[cfg(feature = "skills")]
pub mod write_root;

#[cfg(all(test, feature = "skills"))]
#[path = "e2e_plumbing_tests.rs"]
mod e2e_plumbing_tests;

#[cfg(feature = "skills")]
pub use ops::*;
#[cfg(feature = "skills")]
pub use schemas::{
    all_skills_controller_schemas, all_skills_registered_controllers, skills_schemas,
};

// ---------------------------------------------------------------------------
// Disabled facade — compiled only when the `skills` feature is OFF.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "skills"))]
mod stub;
#[cfg(not(feature = "skills"))]
pub use stub::*;
