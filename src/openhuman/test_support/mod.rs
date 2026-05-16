//! Test-support domain — wipe-and-reset hooks for E2E specs.
//!
//! `openhuman.test_reset` is the one RPC E2E specs call between tests so the
//! running sidecar starts each spec from a pristine state without restarting
//! the process. As new domains add persistent state, extend `rpc::reset` to
//! wipe them too — every new domain that survives a `test_reset` is a leak
//! that will make specs interfere with each other.

pub mod rpc;
mod schemas;

pub use schemas::{
    all_controller_schemas as all_test_support_controller_schemas,
    all_registered_controllers as all_test_support_registered_controllers,
};
