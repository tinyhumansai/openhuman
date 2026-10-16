//! The core's JSON-RPC server.
//!
//! | Module | Owns |
//! |---|---|
//! | `serve` | [`serve`]: bind the listener for a `CoreRuntime` and run until shutdown |
//! | `shims` | the `run_server*` entry points that build a runtime and serve it |
//! | `http` | the axum router and one module per route family |
//! | `auth` | bearer-token route policy over `openhuman::core::auth` |
//! | `classify` | how the `/rpc` handler reports a failed call |
//! | `socketio` | the Socket.IO live-event bridge and `rpc:request` |
//! | `dev_connect` | the debug-only `/dev/connect` handoff |
//! | `cli` | the launcher behind `openhuman-core run` |
//!
//! Dispatch itself is core's
//! (`invoke_method`); every
//! transport here resolves a method through it.

mod auth;
mod classify;
pub(crate) mod cli;
mod dev_connect;
pub(crate) mod http;
mod saas_gateway;
mod serve;
pub(crate) mod shims;
mod socketio;
#[cfg(test)]
pub(crate) mod testing;

pub use http::{build_core_http_router, rpc_handler};
pub use serve::{serve, EmbeddedReadySignal};
pub use shims::{run_server, run_server_headless, run_server_saas};
pub use socketio::publish_companion_state_changed;
