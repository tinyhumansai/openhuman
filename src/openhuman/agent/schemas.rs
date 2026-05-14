use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::agent::profiles::{AgentProfile, AgentProfileStore};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize)]
struct AgentChatParams {
    message: String,
    model_override: Option<String>,
    temperature: Option<f64>,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("chat"),
        schemas("chat_simple"),
        schemas("server_status"),
        schemas("list_definitions"),
        schemas("get_definition"),
        schemas("reload_definitions"),
        schemas("triage_evaluate"),
        schemas("profiles_list"),
        schemas("profile_select"),
        schemas("profile_upsert"),
        schemas("profile_delete"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("chat"),
            handler: handle_chat,
        },
        RegisteredController {
            schema: schemas("chat_simple"),
            handler: handle_chat_simple,
        },
        RegisteredController {
            schema: schemas("server_status"),
            handler: handle_server_status,
        },
        RegisteredController {
            schema: schemas("list_definitions"),
            handler: handle_list_definitions,
        },
        RegisteredController {
            schema: schemas("get_definition"),
            handler: handle_get_definition,
        },
        RegisteredController {
            schema: schemas("reload_definitions"),
            handler: handle_reload_definitions,
        },
        RegisteredController {
            schema: schemas("triage_evaluate"),
            handler: handle_triage_evaluate,
        },
        RegisteredController {
            schema: schemas("profiles_list"),
            handler: handle_profiles_list,
        },
        RegisteredController {
            schema: schemas("profile_select"),
            handler: handle_profile_select,
        },
        RegisteredController {
            schema: schemas("profile_upsert"),
            handler: handle_profile_upsert,
        },
        RegisteredController {
            schema: schemas("profile_delete"),
            handler: handle_profile_delete,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "chat" => ControllerSchema {
            namespace: "agent",
            function: "chat",
            description: "Run one-shot agent chat with optional model overrides.",
            inputs: vec![
                required_string("message", "User message."),
                optional_string("model_override", "Optional model override."),
                optional_f64("temperature", "Optional temperature override."),
            ],
            outputs: vec![json_output("response", "Agent response payload.")],
        },
        "chat_simple" => ControllerSchema {
            namespace: "agent",
            function: "chat_simple",
            description: "Run one-shot lightweight provider chat.",
            inputs: vec![
                required_string("message", "User message."),
                optional_string("model_override", "Optional model override."),
                optional_f64("temperature", "Optional temperature override."),
            ],
            outputs: vec![json_output("response", "Agent response payload.")],
        },
        "server_status" => ControllerSchema {
            namespace: "agent",
            function: "server_status",
            description: "Return core runtime URL and status for agent calls.",
            inputs: vec![],
            outputs: vec![json_output("status", "Agent server status payload.")],
        },
        "list_definitions" => ControllerSchema {
            namespace: "agent",
            function: "list_definitions",
            description: "List all sub-agent definitions in the global registry \
                          (built-ins + custom TOML files under <workspace>/agents/).",
            inputs: vec![],
            outputs: vec![json_output("definitions", "Array of AgentDefinition.")],
        },
        "get_definition" => ControllerSchema {
            namespace: "agent",
            function: "get_definition",
            description: "Fetch a single sub-agent definition by id.",
            inputs: vec![required_string("id", "Definition id (e.g. code_executor).")],
            outputs: vec![json_output("definition", "AgentDefinition payload.")],
        },
        "reload_definitions" => ControllerSchema {
            namespace: "agent",
            function: "reload_definitions",
            description: "Reload custom sub-agent definitions from disk. \
                          NOTE: only takes effect on next process restart in v1 \
                          since the global registry is OnceLock-backed.",
            inputs: vec![],
            outputs: vec![json_output("status", "Reload status payload.")],
        },
        "triage_evaluate" => ControllerSchema {
            namespace: "agent",
            function: "triage_evaluate",
            description: "Run the trigger-triage classifier against a synthetic trigger \
                          payload for testing and replay. Returns the parsed decision \
                          and timing metadata. When dry_run=true the decision is NOT \
                          acted on (no sub-agent dispatch, no events beyond TriggerEvaluated).",
            inputs: vec![
                required_string("source", "Trigger source slug (e.g. 'composio')."),
                optional_string("toolkit", "Toolkit slug (composio-specific)."),
                optional_string("trigger", "Trigger slug (composio-specific)."),
                optional_string("external_id", "Stable per-occurrence id."),
                required_string("display_label", "Human-friendly label."),
                FieldSchema {
                    name: "payload",
                    ty: TypeSchema::Json,
                    comment: "Trigger payload as JSON.",
                    required: true,
                },
                FieldSchema {
                    name: "dry_run",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "When true, skip apply_decision (default: false).",
                    required: false,
                },
            ],
            outputs: vec![json_output("result", "Triage evaluation result.")],
        },
        "profiles_list" => ControllerSchema {
            namespace: "agent",
            function: "profiles_list",
            description: "List persistent agent profiles and the active profile id.",
            inputs: vec![],
            outputs: vec![json_output("profiles", "Agent profile state payload.")],
        },
        "profile_select" => ControllerSchema {
            namespace: "agent",
            function: "profile_select",
            description: "Select the active persistent agent profile.",
            inputs: vec![required_string("profile_id", "Agent profile id.")],
            outputs: vec![json_output(
                "profiles",
                "Updated agent profile state payload.",
            )],
        },
        "profile_upsert" => ControllerSchema {
            namespace: "agent",
            function: "profile_upsert",
            description: "Create or update an agent profile.",
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
        "profile_delete" => ControllerSchema {
            namespace: "agent",
            function: "profile_delete",
            description: "Delete a custom agent profile.",
            inputs: vec![required_string("profile_id", "Agent profile id.")],
            outputs: vec![json_output(
                "profiles",
                "Updated agent profile state payload.",
            )],
        },
        _ => ControllerSchema {
            namespace: "agent",
            function: "unknown",
            description: "Unknown agent controller function.",
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

fn handle_chat(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentChatParams>(params)?;
        let mut config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::local_ai::rpc::agent_chat(
                &mut config,
                &p.message,
                p.model_override,
                p.temperature,
            )
            .await?,
        )
    })
}

fn handle_chat_simple(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentChatParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::local_ai::rpc::agent_chat_simple(
                &config,
                &p.message,
                p.model_override,
                p.temperature,
            )
            .await?,
        )
    })
}

fn handle_server_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::agent_server_status()) })
}

fn handle_list_definitions(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        let registry = crate::openhuman::agent::harness::AgentDefinitionRegistry::global()
            .ok_or_else(|| "AgentDefinitionRegistry not initialised".to_string())?;
        let defs: Vec<&crate::openhuman::agent::harness::AgentDefinition> = registry.list();
        Ok(serde_json::json!({ "definitions": defs }))
    })
}

#[derive(Debug, Deserialize)]
struct GetDefinitionParams {
    id: String,
}

fn handle_get_definition(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<GetDefinitionParams>(params)?;
        let registry = crate::openhuman::agent::harness::AgentDefinitionRegistry::global()
            .ok_or_else(|| "AgentDefinitionRegistry not initialised".to_string())?;
        match registry.get(p.id.trim()) {
            Some(def) => Ok(serde_json::json!({ "definition": def })),
            None => Err(format!("definition '{}' not found", p.id)),
        }
    })
}

fn handle_reload_definitions(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        // The global registry is OnceLock-backed so live reload is a
        // no-op in v1. Reply with a status payload that explains this
        // and tells the caller how to refresh.
        let already_loaded =
            crate::openhuman::agent::harness::AgentDefinitionRegistry::global().is_some();
        Ok(serde_json::json!({
            "status": "noop",
            "registry_initialised": already_loaded,
            "note": "Sub-agent definitions are loaded once at process startup. \
                     Restart the core process to pick up new TOML files under \
                     <workspace>/agents/.",
        }))
    })
}

#[derive(Debug, Deserialize)]
struct TriageEvaluateParams {
    source: String,
    toolkit: Option<String>,
    trigger: Option<String>,
    external_id: Option<String>,
    display_label: String,
    payload: Value,
    dry_run: Option<bool>,
}

fn handle_triage_evaluate(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<TriageEvaluateParams>(params)?;

        tracing::debug!(
            source = %p.source,
            dry_run = p.dry_run.unwrap_or(false),
            has_external_id = p.external_id.is_some(),
            "[rpc][agent] triage_evaluate received"
        );

        // Build a TriggerEnvelope from the RPC params. Source-specific
        // variants are discriminated by `p.source`.
        let envelope = match p.source.as_str() {
            "composio" => {
                tracing::trace!("[rpc][agent] building composio trigger envelope");
                let toolkit = p.toolkit.as_deref().unwrap_or("unknown");
                let trigger = p.trigger.as_deref().unwrap_or("unknown");
                let eid = p.external_id.as_deref().unwrap_or("rpc");
                crate::openhuman::agent::triage::TriggerEnvelope::from_composio(
                    toolkit, trigger, "rpc", eid, p.payload,
                )
            }
            "webhook" => {
                tracing::trace!("[rpc][agent] building webhook trigger envelope");
                let tunnel_id = p.external_id.as_deref().unwrap_or("unknown");
                let method = p.toolkit.as_deref().unwrap_or("POST");
                let path = p.trigger.as_deref().unwrap_or("/");
                crate::openhuman::agent::triage::TriggerEnvelope::from_webhook(
                    tunnel_id, method, path, p.payload,
                )
            }
            "cron" => {
                tracing::trace!("[rpc][agent] building cron trigger envelope");
                let job_id = p.external_id.as_deref().unwrap_or("unknown");
                let job_name = p.display_label.as_str();
                // Preserve the structured payload — extract the output string
                // for the envelope label but keep the full JSON for triage.
                let output = p
                    .payload
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or(job_name);
                crate::openhuman::agent::triage::TriggerEnvelope::from_cron(
                    job_id, job_name, output,
                )
            }
            "external" => {
                tracing::trace!("[rpc][agent] building external trigger envelope");
                let caller_id = p.external_id.as_deref().unwrap_or("unknown");
                let reason = p.display_label.as_str();
                crate::openhuman::agent::triage::TriggerEnvelope::from_external(
                    caller_id, reason, p.payload,
                )
            }
            other => {
                tracing::warn!(source = %other, "[rpc][agent] unsupported trigger source");
                return Err(format!(
                    "unsupported trigger source `{other}` — supported: composio, webhook, cron, external"
                ));
            }
        };

        tracing::debug!(
            source = %envelope.source.slug(),
            external_id_len = envelope.external_id.len(),
            "[rpc][agent] running triage pipeline"
        );

        let outcome = crate::openhuman::agent::triage::run_triage(&envelope)
            .await
            .map_err(|e| format!("triage evaluation failed: {e}"))?;

        let dry_run = p.dry_run.unwrap_or(false);
        match outcome {
            crate::openhuman::agent::triage::TriageOutcome::Decision(run) => {
                if !dry_run {
                    crate::openhuman::agent::triage::apply_decision(run.clone(), &envelope)
                        .await
                        .map_err(|e| format!("apply_decision failed: {e}"))?;
                }

                Ok(serde_json::json!({
                    "decision": run.decision.action.as_str(),
                    "target_agent": run.decision.target_agent,
                    "prompt": run.decision.prompt,
                    "reason": run.decision.reason,
                    "used_local": run.used_local,
                    "latency_ms": run.latency_ms,
                    "resolution_path": run.resolution_path.as_str(),
                    "dry_run": dry_run,
                }))
            }
            crate::openhuman::agent::triage::TriageOutcome::Deferred {
                defer_until_ms,
                reason,
            } => {
                // Deferred outcome: the chain (cloud → cloud-retry →
                // local) all failed; the caller is expected to
                // re-issue this trigger after `defer_until_ms`. No
                // side effects fire on this path.
                Ok(serde_json::json!({
                    "decision": "deferred",
                    "resolution_path": "deferred",
                    "defer_until_ms": defer_until_ms,
                    "reason": reason,
                    "dry_run": dry_run,
                }))
            }
        }
    })
}

fn handle_profiles_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let request_id = format!("profiles-list-{}", uuid::Uuid::new_v4());
        tracing::debug!(
            request_id = %request_id,
            "[rpc][agent][entry] profiles_list"
        );
        let config = config_rpc::load_config_with_timeout().await.map_err(|e| {
            tracing::debug!(
                request_id = %request_id,
                error = %e,
                "[rpc][agent][error] profiles_list load_config"
            );
            e
        })?;
        let state = AgentProfileStore::new(config.workspace_dir)
            .load()
            .map_err(|e| {
                tracing::debug!(
                    request_id = %request_id,
                    error = %e,
                    "[rpc][agent][error] profiles_list load_store"
                );
                e
            })?;
        tracing::debug!(
            request_id = %request_id,
            active_profile_id = %state.active_profile_id,
            profile_count = state.profiles.len(),
            "[rpc][agent][exit] profiles_list"
        );
        Ok(serde_json::json!({
            "profiles": state.profiles,
            "activeProfileId": state.active_profile_id,
        }))
    })
}

fn handle_profile_select(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let request_id = format!("profile-select-{}", uuid::Uuid::new_v4());
        tracing::debug!(
            request_id = %request_id,
            "[rpc][agent][entry] profile_select"
        );
        let p = deserialize_params::<ProfileSelectParams>(params)?;
        tracing::debug!(
            request_id = %request_id,
            profile_id = %p.profile_id,
            "[rpc][agent] profile_select params"
        );
        let config = config_rpc::load_config_with_timeout().await.map_err(|e| {
            tracing::debug!(
                request_id = %request_id,
                profile_id = %p.profile_id,
                error = %e,
                "[rpc][agent][error] profile_select load_config"
            );
            e
        })?;
        let state = AgentProfileStore::new(config.workspace_dir)
            .select(&p.profile_id)
            .map_err(|e| {
                tracing::debug!(
                    request_id = %request_id,
                    profile_id = %p.profile_id,
                    error = %e,
                    "[rpc][agent][error] profile_select store"
                );
                e
            })?;
        tracing::debug!(
            request_id = %request_id,
            profile_id = %p.profile_id,
            active_profile_id = %state.active_profile_id,
            "[rpc][agent][exit] profile_select"
        );
        Ok(serde_json::json!({
            "profiles": state.profiles,
            "activeProfileId": state.active_profile_id,
        }))
    })
}

fn handle_profile_upsert(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let request_id = format!("profile-upsert-{}", uuid::Uuid::new_v4());
        tracing::debug!(
            request_id = %request_id,
            "[rpc][agent][entry] profile_upsert"
        );
        let p = deserialize_params::<ProfileUpsertParams>(params)?;
        tracing::debug!(
            request_id = %request_id,
            profile_id = %p.profile.id,
            agent_id = %p.profile.agent_id,
            "[rpc][agent] profile_upsert params"
        );
        if let Some(registry) = crate::openhuman::agent::harness::AgentDefinitionRegistry::global()
        {
            let agent_id = p.profile.agent_id.trim();
            if !agent_id.is_empty() && registry.get(agent_id).is_none() {
                tracing::debug!(
                    request_id = %request_id,
                    profile_id = %p.profile.id,
                    agent_id,
                    "[rpc][agent][error] profile_upsert unknown_agent"
                );
                return Err(format!("agent definition '{agent_id}' not found"));
            }
            tracing::debug!(
                request_id = %request_id,
                profile_id = %p.profile.id,
                agent_id,
                "[rpc][agent] profile_upsert registry_ok"
            );
        } else {
            tracing::debug!(
                request_id = %request_id,
                "[rpc][agent] profile_upsert registry_unavailable"
            );
        }
        let config = config_rpc::load_config_with_timeout().await.map_err(|e| {
            tracing::debug!(
                request_id = %request_id,
                error = %e,
                "[rpc][agent][error] profile_upsert load_config"
            );
            e
        })?;
        let state = AgentProfileStore::new(config.workspace_dir)
            .upsert(p.profile)
            .map_err(|e| {
                tracing::debug!(
                    request_id = %request_id,
                    error = %e,
                    "[rpc][agent][error] profile_upsert store"
                );
                e
            })?;
        tracing::debug!(
            request_id = %request_id,
            active_profile_id = %state.active_profile_id,
            profile_count = state.profiles.len(),
            "[rpc][agent][exit] profile_upsert"
        );
        Ok(serde_json::json!({
            "profiles": state.profiles,
            "activeProfileId": state.active_profile_id,
        }))
    })
}

fn handle_profile_delete(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let request_id = format!("profile-delete-{}", uuid::Uuid::new_v4());
        tracing::debug!(
            request_id = %request_id,
            "[rpc][agent][entry] profile_delete"
        );
        let p = deserialize_params::<ProfileDeleteParams>(params)?;
        tracing::debug!(
            request_id = %request_id,
            profile_id = %p.profile_id,
            "[rpc][agent] profile_delete params"
        );
        let config = config_rpc::load_config_with_timeout().await.map_err(|e| {
            tracing::debug!(
                request_id = %request_id,
                profile_id = %p.profile_id,
                error = %e,
                "[rpc][agent][error] profile_delete load_config"
            );
            e
        })?;
        let state = AgentProfileStore::new(config.workspace_dir)
            .delete(&p.profile_id)
            .map_err(|e| {
                tracing::debug!(
                    request_id = %request_id,
                    profile_id = %p.profile_id,
                    error = %e,
                    "[rpc][agent][error] profile_delete store"
                );
                e
            })?;
        tracing::debug!(
            request_id = %request_id,
            profile_id = %p.profile_id,
            active_profile_id = %state.active_profile_id,
            profile_count = state.profiles.len(),
            "[rpc][agent][exit] profile_delete"
        );
        Ok(serde_json::json!({
            "profiles": state.profiles,
            "activeProfileId": state.active_profile_id,
        }))
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

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn optional_f64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
        comment,
        required: false,
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

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::TypeSchema;
    use crate::openhuman::agent::profiles::DEFAULT_PROFILE_ID;
    use crate::openhuman::config::TEST_ENV_LOCK as ENV_LOCK;
    use serde_json::json;

    #[test]
    fn controller_schema_inventory_is_stable() {
        let schemas = all_controller_schemas();
        let functions: Vec<_> = schemas.iter().map(|schema| schema.function).collect();
        assert_eq!(
            functions,
            vec![
                "chat",
                "chat_simple",
                "server_status",
                "list_definitions",
                "get_definition",
                "reload_definitions",
                "triage_evaluate",
                "profiles_list",
                "profile_select",
                "profile_upsert",
                "profile_delete",
            ]
        );
        assert_eq!(schemas.len(), all_registered_controllers().len());
    }

    #[test]
    fn schemas_expose_expected_inputs_and_unknown_fallback() {
        let chat = schemas("chat");
        assert_eq!(chat.namespace, "agent");
        assert_eq!(chat.inputs.len(), 3);
        assert!(matches!(chat.inputs[1].ty, TypeSchema::Option(_)));

        let triage = schemas("triage_evaluate");
        assert_eq!(triage.inputs.len(), 7);
        assert!(triage
            .inputs
            .iter()
            .any(|input| input.name == "payload" && input.required));
        assert!(triage
            .inputs
            .iter()
            .any(|input| input.name == "dry_run" && !input.required));

        let profiles = schemas("profiles_list");
        assert_eq!(profiles.inputs.len(), 0);
        let profile_select = schemas("profile_select");
        assert!(profile_select
            .inputs
            .iter()
            .any(|input| input.name == "profile_id" && input.required));

        let unknown = schemas("nope");
        assert_eq!(unknown.function, "unknown");
        assert_eq!(unknown.outputs[0].name, "error");
    }

    #[test]
    fn deserialize_params_and_helpers_cover_success_and_failure_paths() {
        let params = Map::from_iter([
            ("message".into(), Value::String("hello".into())),
            ("model_override".into(), Value::String("gpt".into())),
            ("temperature".into(), json!(0.2)),
        ]);
        let parsed = deserialize_params::<AgentChatParams>(params).expect("valid params");
        assert_eq!(parsed.message, "hello");
        assert_eq!(parsed.model_override.as_deref(), Some("gpt"));
        assert_eq!(parsed.temperature, Some(0.2));

        let err = deserialize_params::<GetDefinitionParams>(Map::new()).expect_err("missing id");
        assert!(err.contains("invalid params"));

        assert!(required_string("id", "x").required);
        assert!(matches!(
            optional_string("id", "x").ty,
            TypeSchema::Option(_)
        ));
        assert!(matches!(
            optional_f64("temperature", "x").ty,
            TypeSchema::Option(_)
        ));
        assert!(matches!(json_output("result", "x").ty, TypeSchema::Json));
    }

    #[tokio::test]
    async fn reload_and_definition_handlers_cover_missing_registry_paths() {
        let reload = handle_reload_definitions(Map::new())
            .await
            .expect("reload handler should always succeed");
        assert_eq!(reload.get("status").and_then(Value::as_str), Some("noop"));
        assert!(reload
            .get("note")
            .and_then(Value::as_str)
            .unwrap()
            .contains("Restart"));

        let list_result = handle_list_definitions(Map::new()).await;
        match list_result {
            Ok(value) => assert!(value.get("definitions").and_then(Value::as_array).is_some()),
            Err(err) => assert!(err.contains("AgentDefinitionRegistry not initialised")),
        }

        let get_err = handle_get_definition(Map::from_iter([(
            "id".into(),
            Value::String("__definitely_missing_definition__".into()),
        )]))
        .await
        .expect_err("missing or unknown definition should error");
        assert!(
            get_err.contains("AgentDefinitionRegistry not initialised")
                || get_err.contains("not found")
        );
    }

    #[tokio::test]
    async fn triage_handler_rejects_unknown_source_and_to_json_maps_outcome() {
        let err = handle_triage_evaluate(Map::from_iter([
            ("source".into(), Value::String("__unknown_source__".into())),
            ("display_label".into(), Value::String("lbl".into())),
            ("payload".into(), json!({})),
        ]))
        .await
        .expect_err("unsupported source should fail before runtime dispatch");
        assert!(err.contains("unsupported trigger source"));

        let value =
            to_json(RpcOutcome::new(json!({ "ok": true }), Vec::new())).expect("json outcome");
        assert_eq!(value["ok"], json!(true));
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
                "allowedTools": ["todowrite"],
                "builtIn": false,
            }),
        )]))
        .await
        .expect("profile upsert");
        assert_eq!(upserted["activeProfileId"], DEFAULT_PROFILE_ID);
        assert!(upserted["profiles"]
            .as_array()
            .expect("profiles array")
            .iter()
            .any(|profile| profile["id"] == "writer"));

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
