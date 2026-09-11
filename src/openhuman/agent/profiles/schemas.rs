//! Controller schemas + handlers for the `profiles` RPC namespace.
//!
//! Methods: `openhuman.profiles_list`, `openhuman.profile_select`,
//! `openhuman.profile_upsert`, `openhuman.profile_delete`.

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::types::AgentProfile;
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list"),
        schemas("select"),
        schemas("upsert"),
        schemas("delete"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list"),
            handler: handle_profiles_list,
        },
        RegisteredController {
            schema: schemas("select"),
            handler: handle_profile_select,
        },
        RegisteredController {
            schema: schemas("upsert"),
            handler: handle_profile_upsert,
        },
        RegisteredController {
            schema: schemas("delete"),
            handler: handle_profile_delete,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list" => ControllerSchema {
            namespace: "profiles",
            function: "list",
            description: "List persistent agent profiles and the active profile id. Each \
                          profile is enriched with resolved read-only path info: soulMdFile \
                          (personalities/<id>/SOUL.md if present) and workspaceDir (the \
                          dedicated workspace when opted in).",
            inputs: vec![],
            outputs: vec![json_output("profiles", "Agent profile state payload.")],
        },
        "select" => ControllerSchema {
            namespace: "profiles",
            function: "select",
            description: "Select the active persistent agent profile.",
            inputs: vec![required_string("profile_id", "Agent profile id.")],
            outputs: vec![json_output(
                "profiles",
                "Updated agent profile state payload.",
            )],
        },
        "upsert" => ControllerSchema {
            namespace: "profiles",
            function: "upsert",
            description: "Create or update an agent profile. The `profile` payload may include \
                          memory_sources, includeAgentConversations, allowedSkills, \
                          allowedMcpServers, composioIntegrations, allowedTools, soulMd, \
                          dedicatedMemory (own memory subtree), and dedicatedWorkspace (own \
                          working dir under action_dir); an omitted/empty allowlist means \"all\".",
            inputs: vec![FieldSchema {
                name: "profile",
                ty: TypeSchema::Json,
                comment: "Agent profile payload.",
                required: true,
            }],
            outputs: vec![json_output(
                "profiles",
                "Updated agent profile state payload.",
            )],
        },
        "delete" => ControllerSchema {
            namespace: "profiles",
            function: "delete",
            description: "Delete a custom agent profile.",
            inputs: vec![required_string("profile_id", "Agent profile id.")],
            outputs: vec![json_output(
                "profiles",
                "Updated agent profile state payload.",
            )],
        },
        _ => ControllerSchema {
            namespace: "profiles",
            function: "unknown",
            description: "Unknown profiles controller function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

#[derive(Debug, Deserialize)]
struct ProfileSelectParams {
    profile_id: String,
}

#[derive(Debug, Deserialize)]
struct ProfileUpsertParams {
    profile: AgentProfile,
}

#[derive(Debug, Deserialize)]
struct ProfileDeleteParams {
    profile_id: String,
}

fn handle_profiles_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(super::ops::list())
}

fn handle_profile_select(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<ProfileSelectParams>(params)?;
        super::ops::select(&p.profile_id).await
    })
}

fn handle_profile_upsert(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<ProfileUpsertParams>(params)?;
        super::ops::upsert(p.profile).await
    })
}

fn handle_profile_delete(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<ProfileDeleteParams>(params)?;
        super::ops::delete(&p.profile_id).await
    })
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
