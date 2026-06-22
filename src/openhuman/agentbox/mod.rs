//! AgentBox marketplace adapter.
//!
//! Exposes `POST /run` and `GET /jobs/{job_id}` over the existing core HTTP
//! server when `OPENHUMAN_AGENTBOX_MODE=1`. Each `/run` invocation drives the
//! full agent runtime; the result is polled via `/jobs/{job_id}`.
//!
//! See `docs/superpowers/specs/2026-06-12-agentbox-marketplace-integration-design.md`.

pub mod env;
pub mod http;
pub mod invoker;
pub mod ops;
pub mod schemas;
pub mod status;
pub mod store;
pub mod types;

pub use env::{agentbox_mode_enabled, register_gmi_provider_if_present};
pub use http::router as agentbox_router;
pub use schemas::{all_agentbox_controller_schemas, all_agentbox_registered_controllers};
pub use status::agentbox_status;
pub use store::JobStore;
pub use types::{AgentBoxProviderInfo, AgentBoxStatus};

#[cfg(test)]
mod disabled_mode_tests;
#[cfg(test)]
mod env_tests;
#[cfg(test)]
mod http_tests;
#[cfg(test)]
mod ops_tests;
#[cfg(test)]
mod store_tests;
#[cfg(test)]
mod test_support;
