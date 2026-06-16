//! Registry: lists of all `memory_tree` controller schemas and registered
//! controller pairs wired into `core::all`.

use crate::core::all::RegisteredController;
use crate::core::ControllerSchema;

use super::definitions::schemas;
use super::handlers::*;

/// All `memory_tree` controller schemas, used by the registry to advertise
/// inputs/outputs to CLI + JSON-RPC consumers.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("ingest"),
        schemas("list_chunks"),
        schemas("get_chunk"),
        schemas("memory_backfill_status"),
        schemas("list_sources"),
        schemas("search"),
        schemas("recall"),
        schemas("entity_index_for"),
        schemas("chunks_for_entity"),
        schemas("top_entities"),
        schemas("chunk_score"),
        schemas("delete_chunk"),
        schemas("delete_source"),
        schemas("graph_export"),
        schemas("obsidian_vault_status"),
        schemas("vault_health_check"),
        schemas("flush_now"),
        schemas("flush_source"),
        schemas("wipe_all"),
        schemas("reset_tree"),
        schemas("pipeline_status"),
        schemas("set_enabled"),
        schemas("smart_walk"),
        schemas("doctor"),
        schemas("retry_failed"),
    ]
}

/// Registered `memory_tree` controllers (schema + handler pairs) wired into
/// `core::all`.
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("ingest"),
            handler: handle_ingest,
        },
        RegisteredController {
            schema: schemas("list_chunks"),
            handler: handle_list_chunks,
        },
        RegisteredController {
            schema: schemas("get_chunk"),
            handler: handle_get_chunk,
        },
        RegisteredController {
            schema: schemas("memory_backfill_status"),
            handler: handle_memory_backfill_status,
        },
        RegisteredController {
            schema: schemas("list_sources"),
            handler: handle_list_sources,
        },
        RegisteredController {
            schema: schemas("search"),
            handler: handle_search,
        },
        RegisteredController {
            schema: schemas("recall"),
            handler: handle_recall,
        },
        RegisteredController {
            schema: schemas("entity_index_for"),
            handler: handle_entity_index_for,
        },
        RegisteredController {
            schema: schemas("chunks_for_entity"),
            handler: handle_chunks_for_entity,
        },
        RegisteredController {
            schema: schemas("top_entities"),
            handler: handle_top_entities,
        },
        RegisteredController {
            schema: schemas("chunk_score"),
            handler: handle_chunk_score,
        },
        RegisteredController {
            schema: schemas("delete_chunk"),
            handler: handle_delete_chunk,
        },
        RegisteredController {
            schema: schemas("delete_source"),
            handler: handle_delete_source,
        },
        RegisteredController {
            schema: schemas("graph_export"),
            handler: handle_graph_export,
        },
        RegisteredController {
            schema: schemas("obsidian_vault_status"),
            handler: handle_obsidian_vault_status,
        },
        RegisteredController {
            schema: schemas("vault_health_check"),
            handler: handle_vault_health_check,
        },
        RegisteredController {
            schema: schemas("flush_now"),
            handler: handle_flush_now,
        },
        RegisteredController {
            schema: schemas("flush_source"),
            handler: handle_flush_source,
        },
        RegisteredController {
            schema: schemas("wipe_all"),
            handler: handle_wipe_all,
        },
        RegisteredController {
            schema: schemas("reset_tree"),
            handler: handle_reset_tree,
        },
        RegisteredController {
            schema: schemas("pipeline_status"),
            handler: handle_pipeline_status,
        },
        RegisteredController {
            schema: schemas("set_enabled"),
            handler: handle_set_enabled,
        },
        RegisteredController {
            schema: schemas("smart_walk"),
            handler: handle_smart_walk,
        },
        RegisteredController {
            schema: schemas("doctor"),
            handler: handle_doctor,
        },
        RegisteredController {
            schema: schemas("retry_failed"),
            handler: handle_retry_failed,
        },
    ]
}
