//! Controller schemas and registration for the `memory` namespace.

mod defs;
mod handlers;

use crate::core::all::RegisteredController;
use crate::core::ControllerSchema;

pub use defs::{schema, FUNCTIONS};

/// Every memory function paired with the handler it dispatches to.
const HANDLERS: &[(&str, crate::core::all::ControllerHandler)] = &[
    ("engines_list", handlers::engines_list),
    ("engine_get", handlers::engine_get),
    ("engine_set", handlers::engine_set),
    ("policy_get", handlers::policy_get),
    ("policy_set", handlers::policy_set),
    ("pack_preview", handlers::pack_preview),
    ("recall", handlers::recall),
    ("fetch", handlers::fetch),
    ("learn", handlers::learn),
    ("forget", handlers::forget),
    ("erase_all", handlers::erase_all),
    ("items_list", handlers::items_list),
    ("explore", handlers::explore),
    ("items_get", handlers::items_get),
    ("agents_list", handlers::agents_list),
    (
        "conversations_backfill_status",
        handlers::conversations_backfill_status,
    ),
    (
        "conversations_backfill_start",
        handlers::conversations_backfill_start,
    ),
    ("brain_sources", handlers::brain_sources),
    ("brain_search", handlers::brain_search),
    ("brain_ingest", handlers::brain_ingest),
    ("brain_forget", handlers::brain_forget),
    ("sources_list", handlers::sources_list),
    ("sources_add", handlers::sources_add),
    ("sources_remove", handlers::sources_remove),
    ("sources_sync", handlers::sources_sync),
    ("jobs_list", handlers::jobs_list),
    ("jobs_run", handlers::jobs_run),
    ("import_scan", handlers::import_scan),
    ("import_start", handlers::import_start),
    ("import_status", handlers::import_status),
    ("import_retry_failed", handlers::import_retry_failed),
    ("migration_scan", handlers::migration_scan),
    ("migration_start", handlers::migration_start),
    ("migration_status", handlers::migration_status),
    ("migration_retry", handlers::migration_retry),
];

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
            handler: handler_for(function).expect("every registered memory function has a handler"),
        })
        .collect()
}

fn handler_for(function: &str) -> Option<crate::core::all::ControllerHandler> {
    HANDLERS
        .iter()
        .find_map(|(name, handler)| (*name == function).then_some(*handler))
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
