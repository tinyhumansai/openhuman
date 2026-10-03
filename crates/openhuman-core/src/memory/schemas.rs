//! Controller schemas and registration for the `memory` namespace.

mod defs;
mod handlers;

use crate::core::all::RegisteredController;
use crate::core::ControllerSchema;

pub use defs::{schema, FUNCTIONS};

/// Every `memory` controller schema.
#[must_use]
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    FUNCTIONS.iter().map(|function| schema(function)).collect()
}

/// Every `memory` controller with its handler.
#[must_use]
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    FUNCTIONS
        .iter()
        .map(|function| RegisteredController {
            schema: schema(function),
            handler: handler_for(function),
        })
        .collect()
}

fn handler_for(function: &str) -> crate::core::all::ControllerHandler {
    match function {
        "engines_list" => handlers::engines_list,
        "engine_get" => handlers::engine_get,
        "engine_set" => handlers::engine_set,
        "recall" => handlers::recall,
        "fetch" => handlers::fetch,
        "learn" => handlers::learn,
        "forget" => handlers::forget,
        "items_list" => handlers::items_list,
        "conversations_get" => handlers::conversations_get,
        "conversations_set" => handlers::conversations_set,
        "sources_list" => handlers::sources_list,
        "sources_add" => handlers::sources_add,
        "sources_remove" => handlers::sources_remove,
        "sources_sync" => handlers::sources_sync,
        "context_get" => handlers::context_get,
        "context_refresh" => handlers::context_refresh,
        "context_set" => handlers::context_set,
        "import_scan" => handlers::import_scan,
        "import_start" => handlers::import_start,
        _ => handlers::import_status,
    }
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
