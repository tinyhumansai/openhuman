//! OpenHuman's thin, policy-owning adapter for the neutral workflow engine.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::json;
use tinyagents_harness::CancellationToken;
use tinyagents_orchestration::workflow::{
    OrchestrationError, WorkflowChildRegistration, WorkflowChildRequest, WorkflowChildResult,
    WorkflowDefinition, WorkflowExecutor,
};

use crate::agent::harness::host_definition::AgentDefinitionRegistry;
use crate::agent::harness::ParentExecutionContext;
use tinyagents_definition::{SandboxMode, ToolScope};
use {
    crate::agent::orchestration::AgentOrchestrationSession,
    crate::agent::orchestration::SpawnAgentRequest, crate::agent::orchestration::WaitAgentOptions,
    tinyagents_tasks::OrchestrationTaskStatus,
};

/// Product policy deliberately remains in OpenHuman. The portable engine sees
/// no safety labels; this adapter admits a catalog definition and keeps its
/// children inside the advertised capability envelope.
#[derive(Clone, Copy)]
pub(super) enum WorkflowSafetyTier {
    ReadOnly,
}

pub(super) fn admit_workflow(definition: &WorkflowDefinition) -> Result<WorkflowSafetyTier> {
    match definition
        .extensions
        .get("safetyTier")
        .and_then(serde_json::Value::as_str)
    {
        Some("read_only") => Ok(WorkflowSafetyTier::ReadOnly),
        Some(other) => Err(anyhow!(
            "workflow '{}' has unsupported safetyTier '{other}'",
            definition.id
        )),
        None => Err(anyhow!(
            "workflow '{}' is missing required safetyTier",
            definition.id
        )),
    }
}

/// Host policy has already admitted the parent context before this executor is
/// created.  This adapter only translates one neutral child request into the
/// existing OpenHuman subagent session and retains BUS/progress behaviour there.
#[derive(Clone)]
pub(super) struct OpenHumanWorkflowExecutor {
    session: AgentOrchestrationSession,
    model_override: Option<String>,
    safety_tier: WorkflowSafetyTier,
    /// The root parent execution context the children are spawned under.
    /// `None` for an executor that only persists the initial run row and never
    /// spawns (`start_workflow_run`'s `initialise`).
    parent: Option<ParentExecutionContext>,
}

impl OpenHumanWorkflowExecutor {
    pub(super) fn new(
        run_id: &str,
        model_override: Option<String>,
        safety_tier: WorkflowSafetyTier,
        parent: Option<ParentExecutionContext>,
    ) -> Self {
        Self {
            session: AgentOrchestrationSession::new(format!("workflow-engine-{run_id}")),
            model_override,
            safety_tier,
            parent,
        }
    }

    fn admit_child(&self, agent_id: &str) -> Result<(), OrchestrationError> {
        match self.safety_tier {
            WorkflowSafetyTier::ReadOnly => {
                let registry = AgentDefinitionRegistry::current().ok_or_else(|| {
                    OrchestrationError(
                        "workflow safety admission requires the agent registry".to_owned(),
                    )
                })?;
                let definition = registry.get(agent_id).ok_or_else(|| {
                    OrchestrationError(format!(
                        "workflow safety admission rejected unknown agent '{agent_id}'"
                    ))
                })?;
                let allowed_named_tool = |tool: &str| {
                    matches!(
                        tool,
                        "web_search_tool" | "web_answer_tool" | "web_contents_tool" | "web_fetch"
                    )
                };
                let read_only = matches!(definition.sandbox_mode, SandboxMode::ReadOnly)
                    || matches!(&definition.tools, ToolScope::Named(tools) if tools.iter().all(|tool| allowed_named_tool(tool)));
                if read_only {
                    Ok(())
                } else {
                    Err(OrchestrationError(format!(
                        "workflow safetyTier read_only rejected agent '{agent_id}' with acting-capable tools"
                    )))
                }
            }
        }
    }
}

#[async_trait]
impl WorkflowExecutor for OpenHumanWorkflowExecutor {
    async fn execute(
        &self,
        request: WorkflowChildRequest,
        cancel: CancellationToken,
        registration: Arc<dyn WorkflowChildRegistration>,
    ) -> Result<WorkflowChildResult, OrchestrationError> {
        self.admit_child(&request.agent_id)?;
        if cancel.is_cancelled() {
            return Err(OrchestrationError(
                "workflow cancelled before child spawn".to_owned(),
            ));
        }
        let parent = self.parent.clone().ok_or_else(|| {
            OrchestrationError(
                "workflow child spawn requires a parent execution context".to_owned(),
            )
        })?;
        let response = self
            .session
            .spawn_agent(
                parent,
                SpawnAgentRequest {
                    agent_id: request.agent_id.clone(),
                    prompt: request.prompt,
                    model: self.model_override.clone(),
                    ..Default::default()
                },
            )
            .await
            .map_err(|error| OrchestrationError(error.to_string()))?;

        // This must happen before awaiting the child: stop() can now cancel
        // every real orchestration id while a worker remains in flight.
        registration.register(response.orchestration_id.clone())?;
        if cancel.is_cancelled() {
            self.session.abort_all().await;
            return Err(OrchestrationError(
                "workflow cancelled after child spawn".to_owned(),
            ));
        }
        let wait = self
            .session
            .wait_agents(WaitAgentOptions {
                orchestration_ids: vec![response.orchestration_id.clone()],
                timeout_ms: None,
            })
            .await
            .map_err(|error| OrchestrationError(error.to_string()))?;
        let child = wait.agents.into_iter().next().ok_or_else(|| {
            OrchestrationError("workflow child returned no terminal snapshot".to_owned())
        })?;
        if child.status != OrchestrationTaskStatus::Completed {
            return Err(OrchestrationError(format!(
                "child '{}' (agent '{}') ended {:?}: {}",
                child.orchestration_id,
                child.agent_id,
                child.status,
                child.error.unwrap_or_default()
            )));
        }
        Ok(WorkflowChildResult {
            child_id: response.orchestration_id,
            // Do not coerce to text: the neutral engine faithfully carries
            // host output JSON into downstream prompts and synthesis.
            output: json!({
                "agentId": child.agent_id,
                "summary": child.result_summary,
            }),
        })
    }

    async fn cancel_children(&self, _child_ids: &[String]) {
        // AgentOrchestrationSession is per workflow run, so this aborts exactly
        // its registered children while retaining existing BUS/progress events.
        self.session.abort_all().await;
    }
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
