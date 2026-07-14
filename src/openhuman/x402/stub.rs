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

// Compiled only in the disabled build (`#[cfg(not(feature = "web3"))] mod stub;`
// in `super`), so a plain `#[cfg(test)]` here runs only when x402 is compiled
// out — it pins the disabled facade's callable no-op + empty-registration
// contract that `core/jsonrpc.rs` boot and `core/all.rs` rely on.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_ledger_is_callable_noop() {
        // Must not panic — the boot path calls this unconditionally (itself
        // runtime-gated on `DomainGroup::Web3`) even in a slim build.
        init_ledger(Path::new("/tmp/openhuman-x402-stub-test"), "session-x");
    }

    #[test]
    fn registration_entry_points_are_empty() {
        assert!(all_x402_registered_controllers().is_empty());
        assert!(all_x402_controller_schemas().is_empty());
    }
}
