//! First-run readiness aggregator (issue #4252).
//!
//! Aggregates existing health / storage / permission / model / Ollama probes
//! into one plain-language report that the onboarding "Readiness" step surfaces
//! so a non-developer can confirm their assistant actually works before
//! finishing setup. See [`ops::evaluate`] for the pure rollup logic and
//! [`ops::gather_inputs`] for the live probe wiring.

pub mod ops;
mod schemas;
pub mod types;

pub use ops as rpc;
pub use types::{
    CheckStatus, ModelConnProbe, OllamaProbe, PermState, PermissionProbe, ReadinessCheck,
    ReadinessInputs, ReadinessReport, StorageProbe,
};

pub use schemas::{
    all_controller_schemas as all_readiness_controller_schemas,
    all_registered_controllers as all_readiness_registered_controllers,
};
