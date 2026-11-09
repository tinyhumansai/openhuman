//! Controller schemas and handler dispatch for the MCP clients domain.
//!
//! Every `schemas(function)` match arm defines the RPC method's input/output
//! shape. Every `handle_*` function deserialises params and delegates to
//! `ops.rs`.

#[cfg(test)]
#[path = "../schemas_tests.rs"]
mod tests;

mod handlers;
mod params;
mod registry;

pub use registry::{all_controller_schemas, all_registered_controllers, schemas};

use crate::config::Config;
use crate::core::all::{
    BoundRegisteredController, ControllerEntry, ControllerFuture, ControllerTask,
};
use crate::core::runtime::DispatchCtx;
use serde_json::Map;

fn registry_get_config(ctx: DispatchCtx) -> ControllerTask<Config> {
    Box::pin(async move { ctx.load_config().await })
}

fn parse_registry_get_params(params: Map<String, serde_json::Value>) -> Result<String, String> {
    handlers::parse_registry_get(&params)
}

fn run_registry_get(_ctx: DispatchCtx, config: Config, qualified_name: String) -> ControllerFuture {
    Box::pin(async move {
        params::to_json(
            crate::mcp::registry::ops::mcp_clients_registry_get(&config, qualified_name).await?,
        )
    })
}

pub(crate) fn registry_get_controller_entry() -> ControllerEntry {
    ControllerEntry::bound(BoundRegisteredController::after_gate(
        registry::schemas("registry_get"),
        registry_get_config,
        parse_registry_get_params,
        run_registry_get,
    ))
}

// Test-only re-exports used by the companion schema tests.
#[cfg(test)]
use params::{read_optional_u32, read_required};

#[cfg(test)]
pub(crate) fn registry_get_parameter_contract() -> crate::core::all::ControllerParamContract {
    crate::core::all::ControllerParamContract {
        method: "mcp_clients.registry_get",
        required: &["qualified_name"],
        optional: &[],
        examples: vec![
            serde_json::json!({"qualified_name":"example/server"}),
            serde_json::json!({"qualified_name":" example/server "}),
        ],
    }
}
