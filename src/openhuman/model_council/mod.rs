//! Model Council — run one question through several models in parallel and
//! synthesize their answers with a chair model.
//!
//! See [`council`] for the deliberation core (pure helpers + the
//! [`council::run_council`] orchestrator) and [`schemas`] for the JSON-RPC
//! controller surface (`openhuman.model_council_run`).

pub mod council;
mod schemas;

pub use schemas::{
    all_controller_schemas as all_model_council_controller_schemas,
    all_registered_controllers as all_model_council_registered_controllers,
};
