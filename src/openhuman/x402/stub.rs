//! Disabled-x402 facade.
//!
//! Compiled only when the `web3` Cargo feature is OFF (see the gate in
//! [`super`]). Only three entry points have always-on callers: `init_ledger`
//! (`core/jsonrpc.rs` boot, itself runtime-gated on `DomainGroup::Web3`) and
//! the controller-registration pair (`core/all.rs`). The `X402RequestTool`
//! registration and the http_request 402-retry path are `#[cfg(feature =
//! "web3")]` at their call sites, so no other x402 surface is referenced when
//! off.
//!
//! Signatures MUST match the real ones exactly; the disabled build
//! (`cargo check --no-default-features --features tokenjuice-treesitter`) is
//! the only thing that catches drift.

use std::path::Path;

use crate::core::all::RegisteredController;
use crate::core::ControllerSchema;

/// No-op: there is no spending ledger to initialise when x402 is compiled out.
/// Mirrors `store::init_global` (re-exported as `init_ledger`). The boot call
/// site is additionally runtime-gated on `DomainGroup::Web3`.
pub fn init_ledger(_workspace_dir: &Path, _session_id: &str) {
    log::debug!("[x402-stub] init_ledger ignored (web3 disabled)");
}

/// No x402 controller schemas when the domain is compiled out.
pub fn all_x402_controller_schemas() -> Vec<ControllerSchema> {
    Vec::new()
}

/// No x402 controllers are registered when the domain is compiled out — the
/// `openhuman.x402_*` RPCs become unknown-method.
pub fn all_x402_registered_controllers() -> Vec<RegisteredController> {
    Vec::new()
}
