//! AgentBox marketplace adapter.
//!
//! Exposes `POST /run` and `GET /jobs/{job_id}` over the existing core HTTP
//! server when `OPENHUMAN_AGENTBOX_MODE=1`. Each `/run` invocation drives the
//! full agent runtime; the result is polled via `/jobs/{job_id}`.
//!
//! See `docs/superpowers/specs/2026-06-12-agentbox-marketplace-integration-design.md`.

pub mod env;
// The `/run` + `/jobs/{id}` HTTP surface is axum-only, so it and the
// `agentbox_router` re-export are exclusive to the `http-server` feature
// (#5048). The axum-free `ops`/`status`/`store`/`schemas`/`invoker` stay
// compiled — the AgentBox controllers + status RPC remain available in slim
// builds; only the router (merged by the gated `core::jsonrpc` router) is shed.
#[cfg(feature = "http-server")]
pub mod http;
pub mod invoker;
pub mod ops;
pub mod schemas;
pub mod status;
pub mod store;
pub mod types;

pub use env::{agentbox_mode_enabled, register_gmi_provider_if_present};
#[cfg(feature = "http-server")]
pub use http::router as agentbox_router;
pub use schemas::{all_agentbox_controller_schemas, all_agentbox_registered_controllers};
pub use status::agentbox_status;
pub use store::JobStore;
pub use types::{AgentBoxProviderInfo, AgentBoxStatus};

// Exercises `build_core_http_router` (axum) — gated in lockstep (#5048).
#[cfg(all(test, feature = "http-server"))]
mod disabled_mode_tests;
#[cfg(test)]
mod env_tests;
// Drives the gated `agentbox::http::router` via `tower::ServiceExt` — gated in
// lockstep (#5048).
#[cfg(all(test, feature = "http-server"))]
mod http_tests;
#[cfg(test)]
mod ops_tests;
#[cfg(test)]
mod store_tests;
#[cfg(test)]
mod test_support;
