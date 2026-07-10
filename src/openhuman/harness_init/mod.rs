//! `harness_init` — first-class orchestration of one-time, first-run setup.
//!
//! On a fresh install several provisioning steps (managed Python runtime,
//! spaCy + model, managed Node runtime) used to run lazily on first use, with
//! no user-visible feedback. This domain runs them eagerly at core startup
//! (spawned non-blocking after the RPC server is ready), tracks per-step
//! progress in an in-memory snapshot, and exposes it over
//! `openhuman.harness_init_status` / `openhuman.harness_init_run` for the
//! frontend initialization screen.
//!
//! Steps delegate to the existing idempotent provisioning code
//! (`runtime_python`, `memory_tree::nlp`, `runtime_node`) — this module
//! orchestrates and reports, it does not reimplement downloads.

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
