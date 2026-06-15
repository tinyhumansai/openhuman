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
            description: "List persistent agent profiles and the active profile id.",
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
                          allowedMcpServers, composioIntegrations, allowedTools, and soulMd; \
                          an omitted/empty allowlist means \"all\".",
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
mod tests {
    use super::*;
    use crate::openhuman::config::TEST_ENV_LOCK as ENV_LOCK;
    use crate::openhuman::profiles::DEFAULT_PROFILE_ID;
    use serde_json::json;

    #[test]
    fn controller_schema_inventory_is_stable() {
        let schemas = all_controller_schemas();
        let functions: Vec<_> = schemas.iter().map(|schema| schema.function).collect();
        assert_eq!(functions, vec!["list", "select", "upsert", "delete"]);
        assert_eq!(schemas.len(), all_registered_controllers().len());
        assert!(schemas.iter().all(|s| s.namespace == "profiles"));
    }

    #[test]
    fn unknown_function_falls_back() {
        let unknown = schemas("nope");
        assert_eq!(unknown.function, "unknown");
        assert_eq!(unknown.outputs[0].name, "error");
    }

    struct WorkspaceEnvGuard {
        previous: Option<std::ffi::OsString>,
    }

    impl WorkspaceEnvGuard {
        fn set(path: &std::path::Path) -> Self {
            let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
            unsafe {
                std::env::set_var("OPENHUMAN_WORKSPACE", path);
            }
            Self { previous }
        }
    }

    impl Drop for WorkspaceEnvGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => unsafe {
                    std::env::set_var("OPENHUMAN_WORKSPACE", value);
                },
                None => unsafe {
                    std::env::remove_var("OPENHUMAN_WORKSPACE");
                },
            }
        }
    }

    #[tokio::test]
    async fn profile_handlers_persist_and_return_profile_state() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let temp = tempfile::tempdir().expect("tempdir");
        let _env = WorkspaceEnvGuard::set(temp.path());

        let upserted = handle_profile_upsert(Map::from_iter([(
            "profile".into(),
            json!({
                "id": "writer",
                "name": "Writer",
                "description": "Draft concise copy",
                "agentId": "orchestrator",
                "modelOverride": "agentic-v1",
                "temperature": 0.2,
                "systemPromptSuffix": "Use a crisp tone.",
                "allowedTools": ["todo"],
                "memorySources": ["slack-eng"],
                "allowedSkills": ["deep-research"],
                "includeAgentConversations": false,
                "builtIn": false,
            }),
        )]))
        .await
        .expect("profile upsert");
        assert_eq!(upserted["activeProfileId"], DEFAULT_PROFILE_ID);
        let writer = upserted["profiles"]
            .as_array()
            .expect("profiles array")
            .iter()
            .find(|profile| profile["id"] == "writer")
            .expect("writer profile present");
        assert_eq!(writer["memorySources"], json!(["slack-eng"]));
        assert_eq!(writer["allowedSkills"], json!(["deep-research"]));
        assert_eq!(writer["includeAgentConversations"], json!(false));

        let selected = handle_profile_select(Map::from_iter([(
            "profile_id".into(),
            Value::String("writer".into()),
        )]))
        .await
        .expect("profile select");
        assert_eq!(selected["activeProfileId"], "writer");

        let listed = handle_profiles_list(Map::new())
            .await
            .expect("profiles list");
        assert_eq!(listed["activeProfileId"], "writer");

        let deleted = handle_profile_delete(Map::from_iter([(
            "profile_id".into(),
            Value::String("writer".into()),
        )]))
        .await
        .expect("profile delete");
        assert_eq!(deleted["activeProfileId"], DEFAULT_PROFILE_ID);
    }

    #[tokio::test]
    async fn profile_upsert_rejects_unknown_registered_agent_id() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = crate::openhuman::agent::harness::AgentDefinitionRegistry::init_global_builtins();

        let err = handle_profile_upsert(Map::from_iter([(
            "profile".into(),
            json!({
                "id": "bad",
                "name": "Bad",
                "description": "",
                "agentId": "__missing_agent__",
                "builtIn": false,
            }),
        )]))
        .await
        .expect_err("unknown agent should fail before store write");
        assert!(err.contains("agent definition"), "err: {err}");
    }
}
