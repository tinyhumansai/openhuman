//! Disabled-skills facade for the skill-runtime domain.
//!
//! Compiled only when the `skills` Cargo feature is OFF (see the gate in
//! [`super`]). Deliberately tiny: every caller of the run machinery
//! (`spawn_workflow_run_background`, `await_run_outcome`, `WorkflowRunStarted`)
//! lives inside code gated by the *same* feature — the `run_workflow` agent
//! tool, the `openhuman.skills_*` handlers, and this domain's own tests — so
//! those vanish together and the stub owes only what always-on code reaches:
//! the controller aggregators (`src/core/all.rs`) and the `tools` module glob
//! (`src/openhuman/tools/mod.rs`).
//!
//! The signatures here MUST match the real ones exactly. The disabled build
//! (`cargo check --no-default-features --features tokenjuice-treesitter`) is
//! the only thing that catches drift.

use crate::core::all::RegisteredController;
use crate::core::ControllerSchema;

/// Always empty: the `openhuman.skill_runtime_*` controllers are compiled out,
/// so they never enter the registry (unknown-method over `/rpc`, absent from
/// `/schema`).
pub fn all_skill_runtime_registered_controllers() -> Vec<RegisteredController> {
    Vec::new()
}

/// Always empty — see [`all_skill_runtime_registered_controllers`].
pub fn all_skill_runtime_controller_schemas() -> Vec<ControllerSchema> {
    Vec::new()
}

// NOTE: no `tools` module here — the `pub use skill_runtime::tools::*` glob in
// `tools/mod.rs` is `#[cfg(feature = "skills")]` instead, mirroring the `voice`
// gate. See the note in `skills/stub.rs`.
