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
//! ### Type carve-out (why `types` + `ops_types` are NOT gated)
//!
//! [`types`] and [`ops_types`] stay compiled in **both** directions. They are
//! inert serde/std-only type definitions with zero coupling to the gated
//! siblings, and they are load-bearing far outside this domain:
//! `tools::traits` re-exports `ToolResult`/`ToolContent` out of [`types`] as
//! the crate's unified tool-result type (`mcp_client`, `runtime_node`, and
//! ~236 files consume it), and `Workflow`/`WorkflowFrontmatter`/
//! `WorkflowScope` from [`ops_types`] appear in always-on agent-harness and
//! prompt signatures. Gating them would take down the entire tool trait
//! system, MCP, and the Node runtime.
//!
//! The generalizable rule: **put inert types in a dep-free submodule and leave
//! it ungated; stub only the behaviour.** The stub below therefore mirrors
//! FUNCTIONS ONLY and re-exports the real types — zero type duplication, so
//! strictly less drift surface than the `voice` stub (which had to re-declare
//! `SttResult` + the `SttProvider` trait because those live inside its gated
//! tree).
//!
//! Signatures in [`stub`] must match the real ones exactly — the disabled
//! build (`cargo check --no-default-features --features tokenjuice-treesitter`)
//! is the only thing that catches drift.

// Type carve-out: always compiled, both feature directions. See module docs.
pub mod ops_types;
pub mod types;

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
pub mod ops_parse;
#[cfg(feature = "skills")]
pub mod preflight;
#[cfg(feature = "skills")]
pub mod registry;
#[cfg(feature = "skills")]
pub mod run_log;
#[cfg(feature = "skills")]
pub mod schemas;
#[cfg(feature = "skills")]
pub mod tools;

#[cfg(all(test, feature = "skills"))]
#[path = "e2e_plumbing_tests.rs"]
mod e2e_plumbing_tests;

#[cfg(all(test, feature = "skills"))]
#[path = "e2e_run_tests.rs"]
mod e2e_run_tests;

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
