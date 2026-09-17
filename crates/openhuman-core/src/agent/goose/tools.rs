use std::{collections::HashSet, sync::Arc};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use goose_agent::{operation::Emitter, tool::ToolProvider};
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock, ErrorData, Tool};
use tinyagents_harness::host::security_gate::{GateDecision, SecurityGate, ToolCallRequest};

use crate::{
    agent::{
        primary_orchestration::capability::{CapabilityPlan, ToolRoute},
        progress::AgentProgress,
        tinyagents::tools::execute_openhuman_tool,
    },
    security::approval::{ApprovalGate, ExecutionOutcome},
};

use super::{types::GooseSession, GooseCheckpointStore};

/// Security seam used by Goose tool dispatch. The production implementation
/// delegates to OpenHuman's existing security/approval gate and completes the
/// same approval audit row after execution.
#[async_trait]
pub trait GooseToolSecurity: Send + Sync {
    async fn authorize(&self, request: &ToolCallRequest) -> Result<GateDecision>;
    fn record_execution(&self, call_id: &str, success: bool, error: Option<&str>);
}

#[async_trait]
impl GooseToolSecurity for crate::agent::tinyagents::host::OpenHumanSecurityGate {
    async fn authorize(&self, request: &ToolCallRequest) -> Result<GateDecision> {
        SecurityGate::authorize_tool(self, request)
            .await
            .map_err(|error| anyhow!(error.to_string()))
    }

    fn record_execution(&self, call_id: &str, success: bool, error: Option<&str>) {
        let Some(request_id) = self.take_audit_request_id(call_id) else {
            return;
        };
        if let Some(gate) = ApprovalGate::try_global() {
            gate.record_execution(
                &request_id,
                if success {
                    ExecutionOutcome::Success
                } else {
                    ExecutionOutcome::Failure
                },
                error,
            );
        }
    }
}

/// Classify balance/quota/credential/provider terminal failures.
pub fn is_terminal_route_failure(message: &str) -> bool {
    if crate::inference::provider::is_budget_exhausted_message(message)
        || crate::inference::provider::is_provider_config_rejection_message(message)
        || crate::core::observability::is_insufficient_credits_message(message)
    {
        return true;
    }
    let norm = message.to_ascii_lowercase().replace('_', " ");
    norm.contains("insufficient credit")
        || norm.contains("insufficient balance")
        || norm.contains("quota exceeded")
        || norm.contains("rate limit")
        || norm.contains("invalid api key")
        || norm.contains("unauthorized")
        || norm.contains("authentication failed")
        || norm.contains("no active credentials")
        || norm.contains("payment required")
        || norm.contains("budget exhausted")
}

/// Verify whether candidate tool route is an authorized same-boundary alternative to the failed route.
/// Never allows cross retrieval/generation, cost, permission, effect, or modality.
pub fn is_same_boundary_alternative(failed: &ToolRoute, candidate: &ToolRoute) -> bool {
    candidate.capability.name != failed.capability.name
        && candidate.capability.operations == failed.capability.operations
        && candidate.capability.modalities == failed.capability.modalities
        && candidate.capability.monetary_boundary == failed.capability.monetary_boundary
        && candidate.capability.side_effect == failed.capability.side_effect
        && candidate.capability.permission <= failed.capability.permission
}

/// Retain only already-authorized same-boundary alternatives when a route fails.
pub fn retain_authorized_alternatives(
    routes: &[ToolRoute],
    unavailable_routes: &[String],
) -> Vec<ToolRoute> {
    if unavailable_routes.is_empty() {
        return routes.to_vec();
    }
    let failed_routes: Vec<&ToolRoute> = routes
        .iter()
        .filter(|r| unavailable_routes.iter().any(|u| u == &r.capability.name))
        .collect();

    routes
        .iter()
        .filter(|route| {
            if unavailable_routes
                .iter()
                .any(|u| u == &route.capability.name)
            {
                return false;
            }
            for failed in &failed_routes {
                if failed
                    .capability
                    .operations
                    .iter()
                    .any(|op| route.capability.operations.contains(op))
                    && !is_same_boundary_alternative(failed, route)
                {
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect()
}

pub struct GooseToolRegistry {
    pub durable_tools: Arc<Vec<Box<dyn crate::tools::Tool>>>,
    pub synthesized_tools: Arc<Vec<Box<dyn crate::tools::Tool>>>,
    pub routes: Vec<ToolRoute>,
    pub enabled_names: HashSet<String>,
}

impl GooseToolRegistry {
    pub fn new(
        durable_tools: Arc<Vec<Box<dyn crate::tools::Tool>>>,
        synthesized_tools: Arc<Vec<Box<dyn crate::tools::Tool>>>,
        routes: Vec<ToolRoute>,
    ) -> Self {
        let enabled_names = CapabilityPlan::derive_enabled_names(&routes);
        Self {
            durable_tools,
            synthesized_tools,
            routes,
            enabled_names,
        }
    }

    pub fn enabled_names(&self) -> &HashSet<String> {
        &self.enabled_names
    }

    pub fn is_enabled(&self, name: &str) -> bool {
        self.enabled_names.contains(name)
    }

    pub fn is_enabled_for_session(&self, name: &str, unavailable_routes: &[String]) -> bool {
        if unavailable_routes.iter().any(|u| u == name) {
            return false;
        }
        self.enabled_names.contains(name)
    }

    pub fn resolve(&self, name: &str) -> Option<&dyn crate::tools::Tool> {
        self.durable_tools
            .iter()
            .find(|tool| tool.name() == name)
            .map(|tool| tool.as_ref())
            .or_else(|| {
                self.synthesized_tools
                    .iter()
                    .find(|tool| tool.name() == name)
                    .map(|tool| tool.as_ref())
            })
    }

    pub fn advertised_tools(&self) -> Result<Vec<Tool>> {
        let mut tools = Vec::new();
        for route in &self.routes {
            if !self.enabled_names.contains(&route.capability.name) {
                continue;
            }
            let Some(tool) = self.resolve(&route.capability.name) else {
                continue;
            };
            tools.push(rmcp_definition(tool)?);
        }
        Ok(tools)
    }

    pub fn advertised_tools_for_session(&self, unavailable_routes: &[String]) -> Result<Vec<Tool>> {
        let retained_routes = retain_authorized_alternatives(&self.routes, unavailable_routes);
        let mut tools = Vec::new();
        for route in &retained_routes {
            if !self.enabled_names.contains(&route.capability.name) {
                continue;
            }
            let Some(tool) = self.resolve(&route.capability.name) else {
                continue;
            };
            tools.push(rmcp_definition(tool)?);
        }
        Ok(tools)
    }
}

pub(super) struct OpenHumanToolProvider {
    pub registry: GooseToolRegistry,
    pub security: Arc<dyn GooseToolSecurity>,
    pub store: Arc<dyn GooseCheckpointStore>,
    pub progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
}

impl OpenHumanToolProvider {
    pub fn new(
        durable_tools: Arc<Vec<Box<dyn crate::tools::Tool>>>,
        synthesized_tools: Arc<Vec<Box<dyn crate::tools::Tool>>>,
        routes: Vec<ToolRoute>,
        security: Arc<dyn GooseToolSecurity>,
        store: Arc<dyn GooseCheckpointStore>,
        progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    ) -> Self {
        Self {
            registry: GooseToolRegistry::new(durable_tools, synthesized_tools, routes),
            security,
            store,
            progress,
        }
    }
}

fn schema_object(
    tool: &dyn crate::tools::Tool,
) -> Result<Arc<serde_json::Map<String, serde_json::Value>>> {
    tool.parameters_schema()
        .as_object()
        .cloned()
        .map(Arc::new)
        .ok_or_else(|| anyhow!("tool '{}' has a non-object parameter schema", tool.name()))
}

fn rmcp_definition(tool: &dyn crate::tools::Tool) -> Result<Tool> {
    Ok(Tool::new(
        tool.name().to_string(),
        tool.description().to_string(),
        schema_object(tool)?,
    ))
}

fn denied_result(reason: String) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(reason)])
}

#[async_trait]
impl ToolProvider<GooseSession> for OpenHumanToolProvider {
    async fn tools(&self, session: &GooseSession) -> Result<Vec<Tool>> {
        self.registry
            .advertised_tools_for_session(&session.checkpoint.unavailable_routes)
    }

    async fn call(
        &self,
        session: &GooseSession,
        request_id: &str,
        call: CallToolRequestParams,
        _emit: &Emitter,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let arguments = serde_json::Value::Object(call.arguments.clone().unwrap_or_default());
        let tool_name = call.name.to_string();
        if !self
            .registry
            .is_enabled_for_session(&tool_name, &session.checkpoint.unavailable_routes)
        {
            return Err(ErrorData::invalid_params(
                format!("tool '{tool_name}' is not enabled or currently unavailable"),
                None,
            ));
        }
        let tool = self.registry.resolve(&tool_name).ok_or_else(|| {
            ErrorData::invalid_params(format!("unknown tool '{tool_name}'"), None)
        })?;

        // The action must have been durably accepted by the previous Goose
        // inference step. Refuse to execute from transient in-memory state.
        let accepted = session
            .checkpoint
            .actions
            .get(request_id)
            .filter(|action| action.observation.is_none())
            .ok_or_else(|| {
                ErrorData::internal_error(
                    format!("tool action '{request_id}' was not durably accepted"),
                    None,
                )
            })?;
        if accepted.tool_name != tool_name || accepted.arguments != arguments {
            return Err(ErrorData::invalid_params(
                format!("persisted tool action '{request_id}' does not match the requested call"),
                None,
            ));
        }

        let security_request =
            ToolCallRequest::new(tool_name.clone(), arguments.clone(), "openhuman-primary")
                .with_call_id(request_id.to_string());
        let decision = self
            .security
            .authorize(&security_request)
            .await
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        if !decision.is_allowed() {
            let reason = decision
                .denial_reason()
                .unwrap_or("Tool execution was not approved")
                .to_string();
            self.security
                .record_execution(request_id, false, Some(&reason));
            return Ok(denied_result(reason));
        }

        let claimed = self
            .store
            .claim_execution(&session.id, request_id)
            .await
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        if !claimed {
            return Ok(denied_result(format!(
                "Tool action '{request_id}' was already claimed by a prior execution; refusing to repeat its side effect"
            )));
        }

        let iteration = session.checkpoint.usage.primary_calls.max(1);
        if let Some(progress) = &self.progress {
            let _ = progress
                .send(AgentProgress::ToolCallStarted {
                    call_id: request_id.to_string(),
                    tool_name: tool_name.clone(),
                    arguments: arguments.clone(),
                    iteration,
                    display_label: tool.display_label(&arguments),
                    display_detail: tool.display_detail(&arguments),
                })
                .await;
        }

        let tiny_call = tinyinference::tool::ToolCall::new(
            request_id.to_string(),
            tool_name.clone(),
            arguments.clone(),
        );
        let result = execute_openhuman_tool(tool, tiny_call, None).await;
        let success = result.error.is_none();
        self.security
            .record_execution(request_id, success, result.error.as_deref());
        if let Some(progress) = &self.progress {
            let _ = progress
                .send(AgentProgress::ToolCallCompleted {
                    call_id: request_id.to_string(),
                    tool_name,
                    success,
                    output_chars: result.content.chars().count(),
                    output: result.content.clone(),
                    arguments: Some(arguments),
                    elapsed_ms: result.elapsed_ms,
                    iteration,
                    failure: None,
                })
                .await;
        }
        Ok(if success {
            CallToolResult::success(vec![ContentBlock::text(result.content)])
        } else {
            denied_result(result.content)
        })
    }
}
