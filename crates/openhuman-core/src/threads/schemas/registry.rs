//! Controller schema and registered-controller lists for the `threads`
//! namespace.

use crate::core::all::{BoundRegisteredController, ControllerEntry, RegisteredController};
use crate::core::ControllerSchema;

use super::handlers::{
    handle_create_new, handle_delete, handle_edit_message, handle_generate_title, handle_goal_get,
    handle_list, handle_message_append, handle_message_update, handle_messages_list, handle_purge,
    handle_regenerate, handle_search, handle_state_get, handle_todos_get, handle_token_usage,
    handle_transcript_get, handle_turn_state_clear, handle_turn_state_get,
    handle_turn_state_get_turn, handle_turn_state_history, handle_turn_state_list,
    handle_update_labels, handle_update_title, handle_update_working_dir, handle_upsert,
    parse_upsert, run_upsert,
};
use super::schema_defs::schemas;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list"),
        schemas("upsert"),
        schemas("create_new"),
        schemas("messages_list"),
        schemas("message_append"),
        schemas("generate_title"),
        schemas("update_labels"),
        schemas("update_title"),
        schemas("update_working_dir"),
        schemas("message_update"),
        schemas("delete"),
        schemas("purge"),
        schemas("turn_state_get"),
        schemas("turn_state_list"),
        schemas("turn_state_history"),
        schemas("turn_state_get_turn"),
        schemas("turn_state_clear"),
        schemas("token_usage"),
        schemas("transcript_get"),
        schemas("goal_get"),
        schemas("todos_get"),
        schemas("state_get"),
        schemas("edit_message"),
        schemas("regenerate"),
        schemas("search"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("upsert"),
            handler: handle_upsert,
        },
        RegisteredController {
            schema: schemas("create_new"),
            handler: handle_create_new,
        },
        RegisteredController {
            schema: schemas("messages_list"),
            handler: handle_messages_list,
        },
        RegisteredController {
            schema: schemas("message_append"),
            handler: handle_message_append,
        },
        RegisteredController {
            schema: schemas("generate_title"),
            handler: handle_generate_title,
        },
        RegisteredController {
            schema: schemas("update_labels"),
            handler: handle_update_labels,
        },
        RegisteredController {
            schema: schemas("update_title"),
            handler: handle_update_title,
        },
        RegisteredController {
            schema: schemas("update_working_dir"),
            handler: handle_update_working_dir,
        },
        RegisteredController {
            schema: schemas("message_update"),
            handler: handle_message_update,
        },
        RegisteredController {
            schema: schemas("delete"),
            handler: handle_delete,
        },
        RegisteredController {
            schema: schemas("purge"),
            handler: handle_purge,
        },
        RegisteredController {
            schema: schemas("turn_state_get"),
            handler: handle_turn_state_get,
        },
        RegisteredController {
            schema: schemas("turn_state_list"),
            handler: handle_turn_state_list,
        },
        RegisteredController {
            schema: schemas("turn_state_history"),
            handler: handle_turn_state_history,
        },
        RegisteredController {
            schema: schemas("turn_state_get_turn"),
            handler: handle_turn_state_get_turn,
        },
        RegisteredController {
            schema: schemas("turn_state_clear"),
            handler: handle_turn_state_clear,
        },
        RegisteredController {
            schema: schemas("token_usage"),
            handler: handle_token_usage,
        },
        RegisteredController {
            schema: schemas("transcript_get"),
            handler: handle_transcript_get,
        },
        RegisteredController {
            schema: schemas("goal_get"),
            handler: handle_goal_get,
        },
        RegisteredController {
            schema: schemas("todos_get"),
            handler: handle_todos_get,
        },
        RegisteredController {
            schema: schemas("state_get"),
            handler: handle_state_get,
        },
        RegisteredController {
            schema: schemas("edit_message"),
            handler: handle_edit_message,
        },
        RegisteredController {
            schema: schemas("regenerate"),
            handler: handle_regenerate,
        },
        RegisteredController {
            schema: schemas("search"),
            handler: handle_search,
        },
    ]
}

pub(crate) fn all_controller_entries() -> Vec<ControllerEntry> {
    all_registered_controllers()
        .into_iter()
        .map(|controller| {
            if controller.schema.function == "upsert" {
                ControllerEntry::bound(BoundRegisteredController::typed(
                    controller.schema,
                    parse_upsert,
                    run_upsert,
                ))
            } else {
                ControllerEntry::legacy(controller)
            }
        })
        .collect::<Vec<_>>()
}
