//! Disabled-x402 facade.
//!
//! Compiled only when the `web3` Cargo feature is off. The controller
//! registration pair remains callable and returns an empty surface. Ledger
//! initialization is the always-on compatibility no-op in the parent module;
//! the real ledger opens lazily through the TinyWallet module.

use crate::core::all::RegisteredController;
use crate::core::ControllerSchema;

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
// contract that `core/runtime/bootstrap.rs` boot and `core/all.rs` rely on.
#[cfg(test)]
#[path = "stub_tests.rs"]
mod tests;
