//! [`all_controller_schemas`] and [`all_registered_controllers`]: the two
//! flat lists — one method name per line — that wire every `composio.*`
//! [`super::definitions::schemas`] entry to its handler and register it
//! with the shared controller registry.

use crate::core::all::{BoundRegisteredController, ControllerEntry, RegisteredController};
use crate::core::ControllerSchema;

use super::definitions::schemas;
use super::handlers_connections::{
    handle_authorize, handle_clear_api_key, handle_delete_connection, handle_get_mode,
    handle_list_agent_ready_toolkits, handle_list_capabilities, handle_list_connections,
    handle_list_toolkits, handle_set_api_key,
};
use super::handlers_identity::{
    handle_get_user_profile, handle_get_user_scopes, handle_refresh_all_identities,
    handle_set_user_scopes,
};
use super::handlers_tools::{
    handle_execute, handle_list_tools, load_execute_config, parse_execute, run_execute,
};
use super::handlers_triggers::{
    handle_create_trigger, handle_disable_trigger, handle_enable_trigger,
    handle_list_available_triggers, handle_list_github_repos, handle_list_trigger_history,
    handle_list_triggers,
};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list_toolkits"),
        schemas("list_capabilities"),
        schemas("list_agent_ready_toolkits"),
        schemas("list_connections"),
        schemas("authorize"),
        schemas("delete_connection"),
        schemas("list_tools"),
        schemas("execute"),
        schemas("list_github_repos"),
        schemas("create_trigger"),
        schemas("get_user_profile"),
        schemas("refresh_all_identities"),
        schemas("list_trigger_history"),
        schemas("get_user_scopes"),
        schemas("set_user_scopes"),
        schemas("list_available_triggers"),
        schemas("list_triggers"),
        schemas("enable_trigger"),
        schemas("disable_trigger"),
        schemas("get_mode"),
        schemas("set_api_key"),
        schemas("clear_api_key"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list_toolkits"),
            handler: handle_list_toolkits,
        },
        RegisteredController {
            schema: schemas("list_capabilities"),
            handler: handle_list_capabilities,
        },
        RegisteredController {
            schema: schemas("list_agent_ready_toolkits"),
            handler: handle_list_agent_ready_toolkits,
        },
        RegisteredController {
            schema: schemas("list_connections"),
            handler: handle_list_connections,
        },
        RegisteredController {
            schema: schemas("authorize"),
            handler: handle_authorize,
        },
        RegisteredController {
            schema: schemas("delete_connection"),
            handler: handle_delete_connection,
        },
        RegisteredController {
            schema: schemas("list_tools"),
            handler: handle_list_tools,
        },
        RegisteredController {
            schema: schemas("execute"),
            handler: handle_execute,
        },
        RegisteredController {
            schema: schemas("list_github_repos"),
            handler: handle_list_github_repos,
        },
        RegisteredController {
            schema: schemas("create_trigger"),
            handler: handle_create_trigger,
        },
        RegisteredController {
            schema: schemas("get_user_profile"),
            handler: handle_get_user_profile,
        },
        RegisteredController {
            schema: schemas("refresh_all_identities"),
            handler: handle_refresh_all_identities,
        },
        RegisteredController {
            schema: schemas("list_trigger_history"),
            handler: handle_list_trigger_history,
        },
        RegisteredController {
            schema: schemas("get_user_scopes"),
            handler: handle_get_user_scopes,
        },
        RegisteredController {
            schema: schemas("set_user_scopes"),
            handler: handle_set_user_scopes,
        },
        RegisteredController {
            schema: schemas("list_available_triggers"),
            handler: handle_list_available_triggers,
        },
        RegisteredController {
            schema: schemas("list_triggers"),
            handler: handle_list_triggers,
        },
        RegisteredController {
            schema: schemas("enable_trigger"),
            handler: handle_enable_trigger,
        },
        RegisteredController {
            schema: schemas("disable_trigger"),
            handler: handle_disable_trigger,
        },
        RegisteredController {
            schema: schemas("get_mode"),
            handler: handle_get_mode,
        },
        RegisteredController {
            schema: schemas("set_api_key"),
            handler: handle_set_api_key,
        },
        RegisteredController {
            schema: schemas("clear_api_key"),
            handler: handle_clear_api_key,
        },
    ]
}

/// Runtime entries preserve the legacy catalog while binding execute's actual
/// runtime parser to the same parser used by schema-example validation.
pub(crate) fn all_controller_entries() -> Vec<ControllerEntry> {
    let execute = BoundRegisteredController::after_gate(
        schemas("execute"),
        load_execute_config,
        parse_execute,
        run_execute,
    );
    all_registered_controllers()
        .into_iter()
        .map(|controller| {
            if controller.schema.method_name() == "composio.execute" {
                ControllerEntry::bound(execute.clone())
            } else {
                ControllerEntry::legacy(controller)
            }
        })
        .collect()
}
