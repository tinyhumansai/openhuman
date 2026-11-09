//! Scope-bound host callbacks for TinyFlows Copilot over TinyBus.

use std::future::Future;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tinybus::Result as BusResult;
use tinyflows_bus::copilot::{self, CopilotHost, PortOperation};

use crate::config::Config;
use crate::core::runtime::{CoreContext, DispatchCtx};
use crate::modules::tinyflows as module;

#[derive(Clone)]
struct FlowsHost {
    config: Arc<Config>,
    context: DispatchCtx,
    runtime: Option<tokio::runtime::Handle>,
}

impl FlowsHost {
    async fn in_context<T, F>(&self, work: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: Future<Output = Result<T, String>> + Send + 'static,
    {
        let context = self.context.clone();
        let work = async move {
            if let Some(core) = context.core().cloned() {
                CoreContext::scope(core, work).await
            } else {
                work.await
            }
        };
        match &self.runtime {
            Some(runtime) => runtime
                .spawn(work)
                .await
                .map_err(|error| format!("the Copilot host callback did not finish: {error}"))?,
            None => work.await,
        }
    }

    async fn dispatch_call_port(&self, request: copilot::PortCallRequest) -> Result<Value, String> {
        let config = Arc::clone(&self.config);
        let context = self.context.clone();
        self.in_context(async move {
            let payload = request.payload;
            macro_rules! wire {
                ($value:expr) => {
                    serde_json::to_value($value).map_err(|error| error.to_string())?
                };
            }
            let value = match request.operation {
                PortOperation::DraftGet => {
                    wire!(ops::flows_draft_get(&config, &required::<String>(&payload, "id")?)?.value)
                }
                PortOperation::DraftCreate => {
                    ops::flows_draft_create_from_copilot(
                        &config,
                        optional_field(&payload, "flow_id")?,
                        required(&payload, "name")?,
                        required(&payload, "graph")?,
                        required(&payload, "origin")?,
                    )?
                }
                PortOperation::DraftUpdate => {
                    wire!(ops::flows_draft_update(
                        &config,
                        &required::<String>(&payload, "id")?,
                        optional_field(&payload, "name")?,
                        optional_field(&payload, "graph")?,
                        None,
                    )?
                    .value)
                }
                PortOperation::FlowsGet => {
                    wire!(ops::flows_get(&config, &required::<String>(&payload, "id")?)
                        .await?
                        .value)
                }
                PortOperation::FlowsList => wire!(ops::flows_list(&config).await?.value),
                PortOperation::FlowsLoadGraph => {
                    wire!(ops::load_flow_graph(&config, &required::<String>(&payload, "id")?)?)
                }
                PortOperation::FlowsCreate => {
                    wire!(ops::flows_create_with_context(
                        &context,
                        &config,
                        required(&payload, "name")?,
                        required(&payload, "graph")?,
                        optional_field::<bool>(&payload, "require_approval")?.unwrap_or(true),
                    )
                    .await?
                    .value)
                }
                PortOperation::FlowsUpdate => {
                    let outcome = ops::flows_update_with_context(
                        &context,
                        &config,
                        &required::<String>(&payload, "id")?,
                        optional_field(&payload, "name")?,
                        optional_field(&payload, "graph")?,
                        None,
                        None,
                    )
                    .await?;
                    json!({"value": outcome.value, "logs": outcome.logs})
                }
                PortOperation::FlowsSetEnabled => {
                    wire!(ops::flows_set_enabled(
                        &config,
                        &required::<String>(&payload, "id")?,
                        required(&payload, "enabled")?,
                    )
                    .await?
                    .value)
                }
                PortOperation::FlowsDuplicate => {
                    wire!(ops::flows_duplicate(&config, &required::<String>(&payload, "id")?)
                        .await?
                        .value)
                }
                PortOperation::FlowsHistory => wire!(ops::flows_get_history(
                    &config,
                    &required::<String>(&payload, "id")?,
                    optional_field::<usize>(&payload, "limit")?.unwrap_or(20),
                )?
                .value),
                PortOperation::FlowsListRuns => wire!(ops::flows_list_runs(
                    &config,
                    &required::<String>(&payload, "flow_id")?,
                    optional_field::<usize>(&payload, "limit")?.unwrap_or(20),
                )
                .await?
                .value),
                PortOperation::FlowsGetRun => {
                    wire!(ops::flows_get_run(&config, &required::<String>(&payload, "run_id")?)
                        .await?
                        .value)
                }
                PortOperation::FlowsResume => {
                    wire!(ops::flows_resume(
                        &context,
                        &config,
                        &required::<String>(&payload, "flow_id")?,
                        &required::<String>(&payload, "run_id")?,
                        optional_field(&payload, "approve")?.unwrap_or_default(),
                        optional_field(&payload, "reject")?.unwrap_or_default(),
                    )
                    .await?
                    .value)
                }
                PortOperation::FlowsCancelRun => {
                    wire!(ops::flows_cancel_run(&config, &required::<String>(&payload, "run_id")?)
                        .await?
                        .value)
                }
                PortOperation::AgentsList => {
                    let agents = crate::agent_registry::list_agents(&context, false)
                        .await
                        .map_err(|error| error.to_string())?;
                    wire!(agents
                        .iter()
                        .map(|agent| {
                            json!({
                                "id": agent.id,
                                "name": agent.name,
                                "description": agent.description,
                                "model": serde_json::to_value(&agent.model).unwrap_or(Value::Null),
                                "tools": serde_json::to_value(&agent.tool_allowlist).unwrap_or(Value::Null),
                                "tags": serde_json::to_value(&agent.tags).unwrap_or(Value::Null),
                            })
                        })
                        .collect::<Vec<_>>())
                }
                PortOperation::SuggestionsUpsert => {
                    json!(ops::flows_suggestions_upsert_from_copilot(
                        &config, payload
                    )?)
                }
                PortOperation::GatesValidate => wire!(ops::flows_validate(&context, &config, payload).await.value),
                PortOperation::GatesRun => {
                    wire!(ops::run_builder_gates_json(&context, &config, payload).await?)
                }
                PortOperation::GatesStrict => {
                    ops::strict_gate(&context, &config, &payload).await?;
                    json!(true)
                }
                PortOperation::GatesEngineCompatibility => {
                    wire!(ops::engine_compatibility_errors_json(&context, &config, payload).await?)
                }
                PortOperation::GatesWiringWarnings => {
                    wire!(ops::wiring_warnings_json(&context, &config, payload).await?)
                }
                PortOperation::GatesApprovalManifest => {
                    wire!(ops::approval_manifest_json(&context, &config, payload).await?)
                }
            };
            Ok(value)
        })
        .await
    }

    async fn dispatch_build_proposal(
        &self,
        request: copilot::BuildProposalRequest,
    ) -> Result<Value, String> {
        let config = Arc::clone(&self.config);
        let context = self.context.clone();
        self.in_context(async move {
            ops::build_builder_proposal_json(
                &context,
                &config,
                request.retry_tool,
                request.name,
                request.graph,
                request.require_approval,
                request.revision,
                request.instruction,
                request.draft_id,
                request.flow_id,
            )
            .await
        })
        .await
    }

    async fn dispatch_run_flow(&self, request: copilot::RunFlowRequest) -> Result<Value, String> {
        let config = Arc::clone(&self.config);
        let context = self.context.clone();
        self.in_context(async move {
            ops::flows_run_copilot(
                &context,
                &config,
                &request.flow_id,
                request.input,
                request.inputs,
            )
            .await
            .map(|outcome| outcome.value)
        })
        .await
    }

    async fn preflight_tool(&self, slug: String, args: Value) -> Result<Value, String> {
        let config = Arc::clone(&self.config);
        self.in_context(async move {
            preflight_mock_tool(&config, &slug, &args).await?;
            // The module discards this acknowledgement and returns only its
            // deterministic MockTools result. Never invoke the real backend.
            Ok(Value::Null)
        })
        .await
    }
}

async fn preflight_mock_tool(config: &Config, slug: &str, args: &Value) -> Result<(), String> {
    if let Some(backend) = crate::flows::caps_host::tools::backend_for(slug) {
        backend
            .preflight(config, slug, args)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn required<T: DeserializeOwned>(value: &Value, key: &str) -> Result<T, String> {
    optional_field(value, key)?.ok_or_else(|| format!("missing `{key}`"))
}

fn optional_field<T: DeserializeOwned>(value: &Value, key: &str) -> Result<Option<T>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|error| format!("bad `{key}`: {error}")),
    }
}

fn bus_error(member: &str, error: impl std::fmt::Display) -> tinybus::Error {
    tracing::debug!(callback = member, %error, "[flows:copilot] host callback failed");
    tinybus::Error::failed(error)
}

struct CopilotCapsHost(FlowsHost);

#[tinybus::interface(name = "ai.tinyhumans.tinyflows.CopilotHost")]
impl FlowsHost {
    async fn call_port(&self, request: copilot::PortCallRequest) -> BusResult<Value> {
        self.dispatch_call_port(request)
            .await
            .map_err(|error| bus_error("CallPort", error))
    }

    async fn build_proposal(&self, request: copilot::BuildProposalRequest) -> BusResult<Value> {
        self.dispatch_build_proposal(request)
            .await
            .map_err(|error| bus_error("BuildProposal", error))
    }

    async fn run_flow(&self, request: copilot::RunFlowRequest) -> BusResult<Value> {
        self.dispatch_run_flow(request)
            .await
            .map_err(|error| bus_error("RunFlow", error))
    }
}

#[tinybus::interface(name = "ai.tinyhumans.tinyflows.CapsHost")]
impl CopilotCapsHost {
    /// Validate mock tool arguments only; sandbox results stay module-owned.
    async fn tool_invoke(
        &self,
        slug: String,
        args: Value,
        _connection: Option<String>,
    ) -> BusResult<Value> {
        self.0
            .preflight_tool(slug, args)
            .await
            .map_err(|error| bus_error("ToolInvoke", error))
    }
}

/// A served Copilot host callback. Its address is passed in the invocation
/// envelope and remains fixed for the entire tool call.
pub(crate) struct HostRegistration {
    _registration: module::ServedObjectRegistration,
    address: CopilotHost,
}

impl HostRegistration {
    pub(crate) fn address(&self) -> CopilotHost {
        self.address.clone()
    }
}

pub(crate) async fn register(
    config: Arc<Config>,
    context: DispatchCtx,
) -> Result<HostRegistration, String> {
    let path = format!(
        "/ai/tinyhumans/openhuman/CopilotHost/h{}",
        uuid::Uuid::new_v4().simple()
    );
    let host = FlowsHost {
        config,
        context,
        runtime: tokio::runtime::Handle::try_current().ok(),
    };
    let registration =
        module::serve_pair(path.clone(), host.clone(), CopilotCapsHost(host)).await?;
    Ok(HostRegistration {
        _registration: registration,
        address: CopilotHost {
            bus_name: module::HOST_BUS_NAME.to_string(),
            object_path: path,
        },
    })
}

#[cfg(test)]
#[path = "builder_host_tests.rs"]
mod tests;

use super::ops;
