//! Startup initialization and progress reporting for the frontend.
//!
//! Language runtimes are provided by the host on `PATH`; this domain does not
//! install Node.js or Python.

pub mod bus;
pub mod ops;
pub mod registry;
pub mod schemas;
pub mod store;
pub mod types;

pub use ops::run_harness_init;
pub use schemas::{
    all_controller_schemas as all_harness_init_controller_schemas,
    all_registered_controllers as all_harness_init_registered_controllers,
};
