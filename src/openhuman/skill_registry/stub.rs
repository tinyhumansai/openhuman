//! Disabled-skills facade for the skill-registry domain.
//!
//! Compiled only when the `skills` Cargo feature is OFF (see the gate in
//! [`super`]). Mirrors only what always-on code reaches: the boot catalog
//! refresh kicked off by `core::runtime::services`, the controller aggregators
//! (`src/core/all.rs`), and the `tools` module glob
//! (`src/openhuman/tools/mod.rs`). Everything else — the catalog store, wire
//! types, and the `skill_setup` agent — is only referenced from code gated by
//! the same feature, so it vanishes alongside.
//!
//! The signatures here MUST match the real ones exactly. The disabled build
//! (`cargo check --no-default-features --features tokenjuice-treesitter`) is
//! the only thing that catches drift.

use crate::core::all::RegisteredController;
use crate::core::ControllerSchema;

/// Always empty: the `openhuman.skill_registry_*` controllers are compiled
/// out, so they never enter the registry (unknown-method over `/rpc`, absent
/// from `/schema`).
pub fn all_skill_registry_registered_controllers() -> Vec<RegisteredController> {
    Vec::new()
}

/// Always empty — see [`all_skill_registry_registered_controllers`].
pub fn all_skill_registry_controller_schemas() -> Vec<ControllerSchema> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// ops::start_boot_catalog_refresh — called unconditionally at core boot from
// `src/core/runtime/services.rs`, so it must stay callable without a `#[cfg]`
// at that always-on site.
// ---------------------------------------------------------------------------

pub mod ops {
    /// No-op: there is no remote skill catalog to warm when the skill domains
    /// are compiled out. Skips the boot-time network fetch entirely.
    pub fn start_boot_catalog_refresh() {
        log::debug!("[skill-registry-stub] start_boot_catalog_refresh skipped (skills disabled)");
    }
}

// NOTE: no `tools` module here — the `pub use skill_registry::tools::*` glob in
// `tools/mod.rs` is `#[cfg(feature = "skills")]` instead, mirroring the `voice`
// gate. See the note in `skills/stub.rs`.
