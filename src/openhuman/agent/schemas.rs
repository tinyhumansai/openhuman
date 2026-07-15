use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize)]
struct AgentChatParams {
    message: String,
    model_override: Option<String>,
    temperature: Option<f64>,
    thread_id: Option<String>,
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
        schemas("graph_topologies"),
        schemas("registry_snapshot"),
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
            schema: schemas("graph_topologies"),
            handler: handle_graph_topologies,
        },
        RegisteredController {
            schema: schemas("registry_snapshot"),
            handler: handle_registry_snapshot,
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
                optional_string(
                    "thread_id",
                    "Optional backend thread id for cache grouping and inference logs.",
                ),
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
                optional_string(
                    "thread_id",
                    "Optional backend thread id for cache grouping and inference logs.",
                ),
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
            description:
                "List safe display metadata for sub-agent definitions in the global registry.",
            inputs: vec![],
            outputs: vec![json_output(
                "definitions",
                "Array of safe AgentDefinitionDisplay payloads; prompt bodies are omitted.",
            )],
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
        "graph_topologies" => ControllerSchema {
            namespace: "agent",
            function: "graph_topologies",
            description: "Export the structure-only topology (Mermaid + JSON + structural \
                          validation report) of every custom tinyagents orchestration graph \
                          for debugging and UI inspection. Structure only — node names, \
                          edges, and routing; never closure bodies or run state.",
            inputs: vec![],
            outputs: vec![json_output(
                "graphs",
                "Object containing graph topology reports and built-in agent graph resolutions.",
            )],
        },
        "registry_snapshot" => ControllerSchema {
            namespace: "agent",
            function: "registry_snapshot",
            description: "Project the tinyagents CapabilityRegistry inventory into a durable, \
                          read-only snapshot: every reachable model / tool / graph / agent \
                          component with its ComponentMetadata (id, kind, description, tags, \
                          aliases), plus a Graphviz DOT rendering. Assembled from the sources \
                          reachable outside an in-flight turn (static model catalog, baseline \
                          tool registry, graph topologies, built-in agents); live per-run \
                          tool/model handles are not included. Metadata only — never run state.",
            inputs: vec![],
            outputs: vec![json_output(
                "components",
                "Array of ComponentMetadata (id/kind/description/tags/aliases). Companion \
                 fields: `counts` (per-kind totals), `dot` (Graphviz DOT), `deferred` (kinds \
                 not fully projected outside a turn).",
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

fn handle_chat(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentChatParams>(params)?;
        let mut config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::local::rpc::agent_chat(
                &mut config,
                &p.message,
                p.model_override,
                p.temperature,
                p.thread_id,
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
            crate::openhuman::inference::local::rpc::agent_chat_simple(
                &config,
                &p.message,
                p.model_override,
                p.temperature,
                p.thread_id,
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
        let defs = crate::openhuman::agent::library::list_definition_metadata().await?;
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

fn handle_graph_topologies(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        let reports = crate::openhuman::tinyagents::all_graph_topologies();
        let agents = crate::openhuman::agent_registry::agents::load_builtins()
            .map_err(|e| format!("loading built-in agent graph resolutions: {e}"))?
            .into_iter()
            .map(|def| {
                let graph = if def.graph.is_default() {
                    "default"
                } else {
                    "custom"
                };
                serde_json::json!({
                    "id": def.id,
                    "graph": graph,
                })
            })
            .collect::<Vec<_>>();
        tracing::debug!(
            graphs = reports.len(),
            agents = agents.len(),
            "[rpc][agent] graph_topologies export"
        );
        let graphs: Vec<Value> = reports
            .iter()
            .map(|r| {
                serde_json::json!({
                    "name": r.name,
                    "ok": r.ok,
                    "errors": r.errors,
                    "warnings": r.warnings,
                    "mermaid": r.mermaid,
                    // The topology JSON is embedded as structured JSON, not a string.
                    "topology": serde_json::from_str::<Value>(&r.json)
                        .unwrap_or(Value::Null),
                })
            })
            .collect();
        Ok(serde_json::json!({ "graphs": graphs, "agents": agents }))
    })
}

/// Read-only projection of the tinyagents `CapabilityRegistry` inventory.
///
/// The per-turn registry (`tinyagents::assemble_turn_harness`) owns live model
/// and tool handles that only exist while a run is in flight and cannot be
/// reached from a standalone RPC. So here we re-project the **same durable
/// descriptor sources** that ARE reachable outside a turn into a
/// [`RegistrySnapshot`], entirely from metadata (no live handles):
///
/// * **Model** — the static cost catalog projection
///   (`cost::catalog::tinyagents_catalog_snapshot`); carries model id, aliases,
///   provider/mode tags.
/// * **Tool** — the baseline tool registry (`tools::default_tools`); names +
///   descriptions. NOTE: the *full* per-agent tool surface (`tools::all_tools`)
///   needs config/memory/audit/action-dir wiring that only exists inside a turn,
///   so only the baseline set is projected here. Deferred — see `deferred` in
///   the response and the migration follow-up.
/// * **Graph** — `tinyagents::all_graph_topologies()` (same source the turn
///   registers as `ComponentKind::Graph` descriptors).
/// * **Agent** — the built-in agent archetypes
///   (`agent_registry::agents::load_builtins()`); id + `when_to_use` blurb.
///
/// Other kinds (Router/Reducer/Store/Middleware/Checkpointer/TaskStore/Listener)
/// are wired per-run and are not enumerable outside a turn; they are reported in
/// `deferred`.
/// Project the user's model registry into runtime-discovered local models for
/// the unified catalog overlay.
///
/// A registry entry is treated as a local runtime model when its `provider`
/// parses as a [`LocalProviderKind`] (`ollama`, `lmstudio`, `mlx`, …). The
/// context window comes from the entry when set, else the runtime profile's
/// default; tool-calling/streaming flags come from the static provider profile
/// (the local runtimes do not advertise per-model capability here). Non-local
/// (cloud/BYOK) rows are skipped — those are owned by the priced catalog layers.
fn local_catalog_models_from_config(
    config: &crate::openhuman::config::Config,
) -> Vec<crate::openhuman::cost::catalog::LocalCatalogModel> {
    use crate::openhuman::inference::local::profile::{
        profile_for_kind, LocalProviderKind, ToolSupport,
    };

    config
        .model_registry
        .iter()
        .filter_map(|entry| {
            let kind = LocalProviderKind::from_str_loose(&entry.provider)?;
            let profile = profile_for_kind(kind);
            let context_window = if entry.context_window > 0 {
                Some(u64::from(entry.context_window))
            } else {
                profile.default_context_window
            };
            Some(crate::openhuman::cost::catalog::LocalCatalogModel {
                provider: entry.provider.clone(),
                model_id: entry.id.clone(),
                context_window,
                tool_calling: matches!(profile.tool_support, ToolSupport::Native),
                streaming: profile.supports_streaming,
            })
        })
        .collect()
}

fn handle_registry_snapshot(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        use tinyagents::registry::{ComponentKind, ComponentMetadata, RegistrySnapshot};

        let mut components: Vec<ComponentMetadata> = Vec::new();

        // ── Models: unified catalog (crate seed + OpenHuman overlay + local) ─
        // Derive runtime-discovered local models from the user's model registry:
        // any entry whose provider is a local runtime (ollama, lmstudio, mlx, …)
        // is overlaid as a free, runtime-profiled catalog entry so local models
        // appear in the one projection alongside priced vendor rows.
        let local_models = match config_rpc::load_config_with_timeout().await {
            Ok(config) => local_catalog_models_from_config(&config),
            Err(err) => {
                tracing::debug!(
                    error = %err,
                    "[registry][rpc][agent] config unavailable; unified catalog omits local models"
                );
                Vec::new()
            }
        };
        let catalog = crate::openhuman::cost::catalog::unified_model_catalog(&local_models);
        let model_count = catalog.models.len();
        for entry in catalog.models {
            let mut meta = ComponentMetadata::new(entry.model_id.clone(), ComponentKind::Model)
                .with_description(format!("{} · {}", entry.provider, entry.mode))
                .with_tag(entry.provider.clone())
                .with_tag(entry.mode.clone());
            meta.aliases = entry.aliases;
            components.push(meta);
        }

        // ── Tools: baseline registry (full per-agent surface deferred) ──────
        let security = std::sync::Arc::new(crate::openhuman::security::SecurityPolicy::default());
        let baseline_tools = crate::openhuman::tools::default_tools(security);
        let tool_count = baseline_tools.len();
        for tool in &baseline_tools {
            components.push(
                ComponentMetadata::new(tool.name().to_string(), ComponentKind::Tool)
                    .with_description(tool.description().to_string()),
            );
        }

        // ── Graphs: structure-only topology reports ─────────────────────────
        let reports = crate::openhuman::tinyagents::all_graph_topologies();
        let graph_count = reports.len();
        for report in &reports {
            let mut meta = ComponentMetadata::new(report.name, ComponentKind::Graph)
                .with_tag(if report.ok { "ok" } else { "invalid" });
            if !report.ok {
                meta = meta.with_description("structural validation failed".to_string());
            }
            components.push(meta);
        }

        // ── Agents: built-in archetypes ─────────────────────────────────────
        let agent_defs = crate::openhuman::agent_registry::agents::load_builtins()
            .map_err(|e| format!("loading built-in agents for registry snapshot: {e}"))?;
        let agent_count = agent_defs.len();
        for def in &agent_defs {
            let mut meta = ComponentMetadata::new(def.id.clone(), ComponentKind::Agent)
                .with_description(def.when_to_use.clone());
            if let Some(display) = &def.display_name {
                meta = meta.with_tag(display.clone());
            }
            components.push(meta);
        }

        // Sort by (kind, id) for stable, diff-friendly output — mirrors the
        // ordering `CapabilityRegistry::snapshot()` produces.
        components.sort_by(|a, b| (a.kind, a.id.0.as_str()).cmp(&(b.kind, b.id.0.as_str())));

        let snapshot = RegistrySnapshot {
            components,
            ..Default::default()
        };
        let dot = snapshot.to_dot();

        tracing::debug!(
            models = model_count,
            tools = tool_count,
            graphs = graph_count,
            agents = agent_count,
            total = snapshot.len(),
            "[registry][rpc][agent] registry_snapshot export"
        );

        Ok(serde_json::json!({
            "components": snapshot.components,
            "counts": {
                "model": model_count,
                "tool": tool_count,
                "graph": graph_count,
                "agent": agent_count,
                "total": snapshot.len(),
            },
            "dot": dot,
            // Kinds not fully enumerable outside an in-flight turn, so callers
            // know the snapshot is a reachable subset rather than the whole
            // per-run registry.
            "deferred": {
                "tool_full_surface": "only baseline tools projected; per-agent all_tools \
                    requires turn-scoped config/memory/audit state",
                "kinds": ["router", "reducer", "store", "middleware", "checkpointer",
                          "task_store", "listener"],
            },
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
                "graph_topologies",
                "registry_snapshot",
            ]
        );
        assert_eq!(schemas.len(), all_registered_controllers().len());
    }

    #[test]
    fn schemas_expose_expected_inputs_and_unknown_fallback() {
        let chat = schemas("chat");
        assert_eq!(chat.namespace, "agent");
        assert_eq!(chat.inputs.len(), 4);
        assert!(matches!(chat.inputs[1].ty, TypeSchema::Option(_)));
        assert!(chat
            .inputs
            .iter()
            .any(|input| input.name == "thread_id" && !input.required));

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
    async fn graph_topologies_handler_exports_structural_reports() {
        let result = handle_graph_topologies(Map::new())
            .await
            .expect("topology export is infallible");
        let graphs = result
            .get("graphs")
            .and_then(Value::as_array)
            .expect("graphs array");
        assert!(!graphs.is_empty(), "at least one custom graph is exported");
        for g in graphs {
            assert!(g.get("name").and_then(Value::as_str).is_some());
            assert_eq!(g.get("ok").and_then(Value::as_bool), Some(true));
            assert!(g
                .get("mermaid")
                .and_then(Value::as_str)
                .unwrap()
                .contains("flowchart"));
            assert!(
                g.get("topology").map(|t| !t.is_null()).unwrap_or(false),
                "topology embeds as structured JSON"
            );
        }
        let names: Vec<_> = graphs
            .iter()
            .filter_map(|g| g.get("name").and_then(Value::as_str))
            .collect();
        assert!(names.contains(&"delegation"), "saw {names:?}");
        assert!(names.contains(&"workflow_runs:scheduler"), "saw {names:?}");
        let agents = result
            .get("agents")
            .and_then(Value::as_array)
            .expect("agents array");
        let orchestrator = agents
            .iter()
            .find(|agent| agent.get("id").and_then(Value::as_str) == Some("orchestrator"))
            .expect("orchestrator graph resolution");
        assert_eq!(
            orchestrator.get("graph").and_then(Value::as_str),
            Some("default")
        );
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
}
