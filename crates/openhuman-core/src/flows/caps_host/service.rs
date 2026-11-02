//! TinyBus `CapsHost` service for one request-scoped flow binding.

use std::sync::Arc;

use serde_json::Value;
use tinyflows_bus::runs;

use crate::flows::http_credentials::HttpCredentialsStore;

#[path = "service_agent.rs"]
mod service_agent;
#[path = "service_helpers.rs"]
mod service_helpers;

use super::binding::CapsHostBinding;
use super::{OpenHumanCode, OpenHumanHttp, OpenHumanLlm, OpenHumanTools};
use service_helpers::{bus_error, memory, to_bus_agent_definition};

pub(crate) struct CapsHostService(Arc<CapsHostBinding>);

impl CapsHostService {
    pub(crate) fn new(binding: Arc<CapsHostBinding>) -> Self {
        Self(binding)
    }
}

#[tinybus::interface(name = "ai.tinyhumans.tinyflows.CapsHost")]
impl CapsHostService {
    async fn llm_complete(&self, request: Value, conn: Option<String>) -> tinybus::Result<Value> {
        let binding = Arc::clone(&self.0);
        let scope = Arc::clone(&binding);
        scope
            .within(async move {
                OpenHumanLlm {
                    ctx: binding.ctx.clone(),
                    config: Arc::clone(&binding.config),
                }
                .complete(request, conn.as_deref())
                .await
            })
            .await
            .map_err(bus_error)
    }

    async fn tool_invoke(
        &self,
        slug: String,
        args: Value,
        conn: Option<String>,
    ) -> tinybus::Result<Value> {
        let binding = Arc::clone(&self.0);
        let scope = Arc::clone(&binding);
        scope
            .within(async move {
                OpenHumanTools {
                    ctx: binding.ctx.clone(),
                    config: Arc::clone(&binding.config),
                    security: Arc::clone(&binding.security),
                    approval: binding.approval.clone(),
                }
                .invoke(&slug, args, conn.as_deref())
                .await
            })
            .await
            .map_err(bus_error)
    }

    async fn http_request(&self, request: Value, conn: Option<String>) -> tinybus::Result<Value> {
        let binding = Arc::clone(&self.0);
        let scope = Arc::clone(&binding);
        scope
            .within(async move {
                OpenHumanHttp {
                    ctx: binding.ctx.clone(),
                    config: Arc::clone(&binding.config),
                    security: Arc::clone(&binding.security),
                    http_config: binding.config.http_request.clone(),
                    http_creds: Arc::new(HttpCredentialsStore::from_config(&binding.config)),
                    approval: binding.approval.clone(),
                }
                .request(request, conn.as_deref())
                .await
            })
            .await
            .map_err(bus_error)
    }

    async fn code_run(&self, request: runs::CodeRunRequest) -> tinybus::Result<Value> {
        let binding = Arc::clone(&self.0);
        let scope = Arc::clone(&binding);
        scope
            .within(async move {
                OpenHumanCode {
                    config: Arc::clone(&binding.config),
                    ctx: binding.ctx.clone(),
                    security: Arc::clone(&binding.security),
                    approval: binding.approval.clone(),
                }
                .run(request.language, &request.source, request.input)
                .await
            })
            .await
            .map_err(bus_error)
    }

    async fn shell_run(
        &self,
        _request: runs::ShellRunRequest,
    ) -> tinybus::Result<runs::ShellRunOutcome> {
        Err(tinybus::Error::failed(
            "shell execution is disabled for workflow runs",
        ))
    }

    async fn memory_recall(
        &self,
        scope: String,
        query: String,
        opts: Value,
    ) -> tinybus::Result<Value> {
        let binding = Arc::clone(&self.0);
        let context = Arc::clone(&binding);
        context
            .within(async move { memory(&binding).recall(&scope, &query, opts).await })
            .await
            .map_err(bus_error)
    }

    async fn memory_flavour(&self, slug: String) -> tinybus::Result<Value> {
        let binding = Arc::clone(&self.0);
        let scope = Arc::clone(&binding);
        scope
            .within(async move { memory(&binding).flavour(&slug).await })
            .await
            .map_err(bus_error)
    }

    async fn memory_people(&self, query: Option<String>) -> tinybus::Result<Value> {
        let binding = Arc::clone(&self.0);
        let scope = Arc::clone(&binding);
        scope
            .within(async move { memory(&binding).people(query.as_deref()).await })
            .await
            .map_err(bus_error)
    }

    async fn memory_remember(
        &self,
        scope: String,
        key: String,
        value: Value,
    ) -> tinybus::Result<()> {
        let binding = Arc::clone(&self.0);
        let context = Arc::clone(&binding);
        context
            .within(async move { memory(&binding).remember(&scope, &key, value).await })
            .await
            .map_err(bus_error)
    }

    async fn memory_forget(&self, scope: String, key: String) -> tinybus::Result<()> {
        let binding = Arc::clone(&self.0);
        let context = Arc::clone(&binding);
        context
            .within(async move { memory(&binding).forget(&scope, &key).await })
            .await
            .map_err(bus_error)
    }

    async fn agent_run(
        &self,
        request: runs::AgentRunRequest,
    ) -> tinybus::Result<runs::AgentRunHandle> {
        if request
            .identity
            .run_id
            .as_deref()
            .is_some_and(|run_id| run_id != self.0.run_id)
        {
            return Err(tinybus::Error::failed(
                "agent request belongs to a different workflow run",
            ));
        }
        if !valid_agent_request_id(&request.client_request_id) {
            return Err(tinybus::Error::failed(
                "AgentRun requires a nonempty bounded client_request_id",
            ));
        }
        let state = Arc::new(tokio::sync::Mutex::new(
            super::agent::AgentRunState::default(),
        ));
        let binding = Arc::clone(&self.0);
        let cancellation = binding.cancellation.child_token();
        let admission = binding
            .agent_runs
            .lock()
            .await
            .admit(request.client_request_id.clone(), state, cancellation)
            .map_err(tinybus::Error::failed)?;
        if admission.cancelled_before_admission {
            let mut state = admission.entry.state.lock().await;
            state.error = Some("agent request was cancelled before admission".to_owned());
            state.done = true;
        }
        let handle = admission.entry.handle.clone();
        if admission.start_task {
            let task_binding = Arc::clone(&binding);
            let entry = admission.entry.clone();
            tokio::spawn(async move {
                let _ = service_agent::run_agent(
                    task_binding,
                    request,
                    Arc::clone(&entry.state),
                    entry.cancellation,
                )
                .await;
            });
        }
        Ok(runs::AgentRunHandle { handle })
    }

    /// Cancel one host task; the id is scoped to this callback binding and
    /// grants no authority outside its object path.
    async fn agent_cancel(&self, request: runs::AgentCancelRequest) -> tinybus::Result<()> {
        if !valid_agent_request_id(&request.client_request_id) {
            return Err(tinybus::Error::failed(
                "AgentCancel requires a nonempty bounded client_request_id",
            ));
        }
        self.0
            .agent_runs
            .lock()
            .await
            .cancel(request.client_request_id);
        Ok(())
    }

    async fn agent_events(
        &self,
        request: runs::AgentEventsRequest,
    ) -> tinybus::Result<runs::AgentEventsResponse> {
        let state = self
            .0
            .agent_runs
            .lock()
            .await
            .state_for_handle(&request.handle)
            .ok_or_else(|| tinybus::Error::failed("unknown agent run handle"))?;
        let state = state.lock().await;
        if let Some(error) = &state.error {
            return Err(tinybus::Error::failed(error));
        }
        Ok(runs::AgentEventsResponse {
            events: state
                .events
                .iter()
                .filter(|event| event.seq > request.cursor)
                .take(runs::MAX_AGENT_EVENTS_PAGE)
                .cloned()
                .collect(),
            done: state.done,
            outcome: state.done.then(|| state.outcome.clone()).flatten(),
        })
    }

    async fn resolve_agent(&self, id: String) -> tinybus::Result<Option<runs::AgentDefinition>> {
        let binding = Arc::clone(&self.0);
        let scope = Arc::clone(&binding);
        scope
            .within(async move {
                crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global(
                    &binding.config.workspace_dir,
                )
                .map_err(|error| error.to_string())?;
                let registry =
                    crate::agent_registry::host_definition::AgentDefinitionRegistry::current();
                registry
                    .and_then(|registry| registry.get(&id).cloned())
                    .map(|definition| to_bus_agent_definition(&definition))
                    .transpose()
            })
            .await
            .map_err(bus_error)
    }

    async fn list_agents(&self) -> tinybus::Result<Vec<runs::AgentDefinition>> {
        let binding = Arc::clone(&self.0);
        let scope = Arc::clone(&binding);
        scope
            .within(async move {
                crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global(
                    &binding.config.workspace_dir,
                )
                .map_err(|error| error.to_string())?;
                let registry =
                    crate::agent_registry::host_definition::AgentDefinitionRegistry::current();
                match registry {
                    Some(registry) => registry
                        .list()
                        .into_iter()
                        .map(to_bus_agent_definition)
                        .collect(),
                    None => Err("agent definition registry is unavailable".to_owned()),
                }
            })
            .await
            .map_err(bus_error)
    }

    async fn resolve_context(
        &self,
        _request: runs::ResolveContextRequest,
    ) -> tinybus::Result<Option<runs::ContextBlock>> {
        Ok(None)
    }

    async fn resolve_tools(
        &self,
        _request: runs::ResolveToolsRequest,
    ) -> tinybus::Result<Vec<runs::ToolDescriptor>> {
        Ok(Vec::new())
    }

    async fn resolve_workdir(
        &self,
        _request: runs::ResolveWorkdirRequest,
    ) -> tinybus::Result<runs::WorkdirCheck> {
        Ok(runs::WorkdirCheck::Unmanaged)
    }

    async fn approval_decide(
        &self,
        request: runs::ApprovalRequest,
    ) -> tinybus::Result<runs::ApprovalOutcome> {
        if request
            .run_id
            .as_deref()
            .is_some_and(|run_id| run_id != self.0.run_id)
        {
            return Err(tinybus::Error::failed(
                "approval request belongs to a different workflow run",
            ));
        }
        // OpenHuman persists graph-review rows through the module's parked run
        // lifecycle. This callback preserves its idempotent pending contract;
        // acting-tool approvals still use the host SecurityApprovalGate.
        Ok(runs::ApprovalOutcome::Pending)
    }

    async fn approval_cancel(&self, _request: runs::ApprovalCancelRequest) -> tinybus::Result<()> {
        // The module owns durable graph-review row retirement and sends this
        // callback only as lifecycle notification.
        Ok(())
    }
}

fn valid_agent_request_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
