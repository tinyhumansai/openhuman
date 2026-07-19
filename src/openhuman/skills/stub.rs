//! Disabled-skills facade.
//!
//! Compiled only when the `skills` Cargo feature is OFF (see the gate in
//! [`super`]). It mirrors the subset of the real `skills` public surface that
//! always-on callers depend on, with no-op / empty bodies so the crate still
//! compiles, boots, and serves `/rpc` without the skills domains.
//!
//! Unlike the `voice` stub, this one mirrors **functions only**: the domain's
//! types live in the ungated [`super::types`] / [`super::ops_types`] carve-out
//! and are re-exported verbatim below, so there is zero type duplication and
//! correspondingly less drift surface.
//!
//! The signatures here MUST match the real ones exactly (return types
//! included). The disabled build
//! (`cargo check --no-default-features --features tokenjuice-treesitter`) is
//! the only thing that catches drift — if a real signature changes, update the
//! mirror below until that build is green again.

use std::path::Path;

use crate::core::all::RegisteredController;
use crate::core::ControllerSchema;

// ---------------------------------------------------------------------------
// Type surface — re-exported from the ungated carve-out, NOT re-declared.
// Mirrors the `pub use ops::*` → `pub use super::ops_types::{…}` chain that the
// real build exposes at the `skills` root (`skills::Workflow`, etc.).
// ---------------------------------------------------------------------------

pub use super::ops_types::{
    Workflow, WorkflowFrontmatter, WorkflowScope, MAX_WORKFLOW_RESOURCE_BYTES,
};

// ---------------------------------------------------------------------------
// Discovery surface (mirrors `ops_discover::*` re-exported at the skills root)
// ---------------------------------------------------------------------------

/// Always empty: no skills are discovered when the domain is compiled out.
/// Callers (agent harness, channel startup, prompt rendering) treat an empty
/// catalog as "user has no skills installed", which is the correct degraded
/// behaviour.
pub fn load_workflow_metadata(_workspace_dir: &Path) -> Vec<Workflow> {
    log::debug!("[skills-stub] load_workflow_metadata -> [] (skills disabled)");
    Vec::new()
}

/// No-op success: with skills compiled out there is no skills directory to
/// provision, and workspace bootstrap must not fail because of it.
pub fn init_workflows_dir(_workspace_dir: &Path) -> Result<(), String> {
    log::debug!("[skills-stub] init_workflows_dir skipped (skills disabled)");
    Ok(())
}

// ---------------------------------------------------------------------------
// Controller aggregators — empty so `src/core/all.rs` needs no `#[cfg]`.
// ---------------------------------------------------------------------------

/// Always empty: the `openhuman.skills_*` controllers are compiled out, so
/// they never enter the registry (unknown-method over `/rpc`, absent from
/// `/schema`).
pub fn all_skills_registered_controllers() -> Vec<RegisteredController> {
    Vec::new()
}

/// Always empty — see [`all_skills_registered_controllers`].
pub fn all_skills_controller_schemas() -> Vec<ControllerSchema> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// registry::prune_legacy_default_workflows
// ---------------------------------------------------------------------------

pub mod registry {
    use std::path::Path;

    /// No-op: the legacy bundled-skill prune only removes directories this
    /// domain created, and it never ran when skills are compiled out.
    pub fn prune_legacy_default_workflows(_workspace_dir: &Path) {
        log::debug!("[skills-stub] prune_legacy_default_workflows skipped (skills disabled)");
    }
}

// ---------------------------------------------------------------------------
// bus::{ensure_triggered_workflow_subscriber, register_workflow_cleanup_subscriber}
// ---------------------------------------------------------------------------

pub mod bus {
    /// No-op: there are no triggered skills to subscribe to the event bus for.
    pub fn ensure_triggered_workflow_subscriber(_workspace: &std::path::Path) {
        log::debug!("[skills-stub] ensure_triggered_workflow_subscriber skipped (skills disabled)");
    }

    /// No-op: no skill run directories exist to clean up.
    pub fn register_workflow_cleanup_subscriber() {}
}

// NOTE: no `tools` module here. The `pub use skills::tools::*` glob in
// `tools/mod.rs` is `#[cfg(feature = "skills")]` instead — mirroring the
// `voice` gate's handling of the `audio_toolkit::tools::*` glob. An empty stub
// module would re-export nothing and trip `unused_imports` at the glob site.
