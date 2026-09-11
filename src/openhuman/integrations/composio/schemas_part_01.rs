use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

#[derive(Debug, serde::Deserialize)]
struct TriggerHistoryParams {
    limit: Option<usize>,
}

#[derive(Debug, serde::Deserialize)]
struct ListGithubReposParams {
    connection_id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct CreateTriggerParams {
    slug: String,
    connection_id: Option<String>,
    trigger_config: Option<Value>,
}

#[derive(Debug, serde::Deserialize)]
struct ListAvailableTriggersParams {
    toolkit: String,
    connection_id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct ListTriggersParams {
    toolkit: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct EnableTriggerParams {
    connection_id: String,
    slug: String,
    trigger_config: Option<Value>,
}

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
        schemas("sync"),
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
            schema: schemas("sync"),
            handler: handle_sync,
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
