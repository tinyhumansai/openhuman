//! Disabled-web3 facade.
//!
//! Compiled only when the `web3` Cargo feature is OFF (see the gate in
//! [`super`]). The high-level swap/bridge/dapp surface has no always-on
//! callers other than the central registration points in `core/all.rs`
//! (controllers) and `tools/ops.rs` (agent tools), so the stub only needs to
//! return empty collections from those three entry points — no per-call
//! `#[cfg]` at the call sites.
//!
//! Signatures MUST match the real ones exactly; the disabled build
//! (`cargo check --no-default-features --features tokenjuice-treesitter`) is
//! the only thing that catches drift.

use crate::core::all::RegisteredController;
use crate::core::ControllerSchema;
use crate::openhuman::tools::traits::Tool;

/// No web3 controller schemas when the domain is compiled out.
pub fn all_web3_controller_schemas() -> Vec<ControllerSchema> {
    Vec::new()
}

/// No web3 controllers are registered when the domain is compiled out — the
/// `openhuman.web3_*` RPCs become unknown-method.
pub fn all_web3_registered_controllers() -> Vec<RegisteredController> {
    Vec::new()
}

/// No web3 agent tools (swap/bridge/dapp) when the domain is compiled out.
pub fn all_web3_agent_tools() -> Vec<Box<dyn Tool>> {
    Vec::new()
}

// Compiled only in the disabled build (`#[cfg(not(feature = "web3"))] mod stub;`
// in `super`), so a plain `#[cfg(test)]` runs only when web3 is compiled out —
// it pins the empty controller/tool surface that `core/all.rs` + `tools/ops.rs`
// consume without per-call `#[cfg]`.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_entry_points_are_empty() {
        assert!(all_web3_registered_controllers().is_empty());
        assert!(all_web3_controller_schemas().is_empty());
    }

    #[test]
    fn agent_tools_are_absent() {
        assert!(all_web3_agent_tools().is_empty());
    }
}
