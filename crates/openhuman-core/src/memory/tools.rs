//! The host adapter for TinyMemory's single `memory` agent tool.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinymemory_api::{Namespace, Reach, ThreadWindow, ToolCallRef};
use tinymemory_tools::{
    concurrency_for, is_write_action, permission_for, CallContext, ConcurrencyClass, MemoryTools,
    PermissionClass, Refusal, RunBudget, SingleTool, ToolScope,
};
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult, ToolRunContext};

use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::core::runtime::DispatchCtx;

/// The tool's name.
pub const MEMORY_TOOL_NAME: &str = "memory";
const REFUSAL_METADATA_KEY: &str = "openhuman.memory_budget";
const CONTINUE_WITHOUT_MEMORY: &str = "Continue answering the user's request without memory. Do not retry memory in this run. A timed-out write may already have been accepted; do not claim it failed or repeat it.";
#[cfg(test)]
const LEARN_STATUS: &str = "queued; not yet saved to memory";

/// What the host knows about a call that the model does not set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallFacts {
    /// The agent's working folder.
    pub workspace: Option<String>,
    /// The thread the call runs in.
    pub thread_id: Option<String>,
    /// The agent that made the call.
    pub agent_id: Option<String>,
    /// The provider-assigned tool-call id.
    pub tool_call_id: Option<String>,
    /// The calling thread's turns still verbatim in the run's prompt.
    pub in_prompt: Option<ThreadWindow>,
    /// The chat the call's turn is surfaced in.
    pub chat: Option<crate::security::approval::ApprovalChatContext>,
    /// Where the agent's learnings go.
    pub namespace: Namespace,
    /// What the agent may read and forget.
    pub reach: Reach,
}

impl CallFacts {
    /// Gathers trusted call facts, falling back to `config.action_dir`.
    #[must_use]
    pub fn gather(config: &Config, context: Option<&dyn ToolRunContext>) -> Self {
        let workspace = context
            .and_then(ToolRunContext::workspace_root)
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| config.action_dir.clone());
        let identity =
            crate::agent::tinyagents::host::run_context::memory_identity_for_tool(context)
                .unwrap_or_else(|| {
                    crate::agent::tinyagents::host::run_context::parent_agent_id_for_tool(context)
                        .map_or_else(
                            tinymemory_tools::layout::identity::Identity::root,
                            |parent_agent_id| {
                                tinymemory_tools::layout::identity::Identity::agent(
                                    &parent_agent_id,
                                )
                            },
                        )
                });
        Self {
            workspace: Some(workspace.display().to_string()),
            thread_id: context
                .and_then(ToolRunContext::thread_id)
                .map(str::to_string),
            tool_call_id: crate::tools::host_extensions::tool_call_id(context),
            chat: crate::agent::tinyagents::host::run_context::approval_for_tool(context).chat,
            in_prompt: crate::agent::tinyagents::host::run_context::memory_in_prompt_for_tool(
                context,
            ),
            ..Self::of(config, &identity)
        }
    }

    /// The facts of a call made by `identity`, without a run context.
    #[must_use]
    pub fn of(config: &Config, identity: &tinymemory_tools::layout::identity::Identity) -> Self {
        Self::of_resolved(&identity.resolve(&crate::memory::scope::identity_pins(config)))
    }

    /// The facts of a call made by an already resolved identity.
    #[must_use]
    pub fn of_resolved(resolved: &tinymemory_tools::layout::identity::ResolvedIdentity) -> Self {
        Self {
            agent_id: Some(resolved.agent_id.clone()),
            namespace: resolved.layout.learnings().clone(),
            reach: Reach::subtree(resolved.root().clone()),
            ..Self::default()
        }
    }

    pub(crate) fn native_scope(&self, config: &Config) -> ToolScope {
        ToolScope {
            place: self.namespace.clone(),
            reach: Some(self.reach.clone()),
            writes: true,
            zone: Some(config.time_zone()),
            workspace: self.workspace.clone(),
            thread_id: self.thread_id.clone(),
            agent_id: self.agent_id.clone(),
            tool_call: Some(ToolCallRef {
                name: MEMORY_TOOL_NAME.to_string(),
                id: self.tool_call_id.clone(),
            }),
        }
    }
}

/// OpenHuman's monotonic Tokio clock for TinyMemory's runtime-neutral budget.
struct TokioClock {
    origin: tokio::time::Instant,
}

impl Default for TokioClock {
    fn default() -> Self {
        Self {
            origin: tokio::time::Instant::now(),
        }
    }
}

impl tinymemory_tools::BudgetClock for TokioClock {
    fn now(&self) -> Duration {
        tokio::time::Instant::now().duration_since(self.origin)
    }

    fn sleep_until(&self, deadline: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(tokio::time::sleep_until(self.origin + deadline))
    }
}

/// The single `memory` tool exposed to the agent.
pub struct MemoryTool {
    config: Arc<Config>,
    budget: RunBudget,
}

impl MemoryTool {
    /// A tool bound to `config`'s configured memory engine.
    #[must_use]
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            budget: RunBudget::new(Arc::new(TokioClock::default())),
        }
    }

    async fn call_with_facts(
        &self,
        args: Value,
        dispatch: DispatchCtx,
        facts: CallFacts,
        run_id: Option<&str>,
    ) -> ToolResult {
        let call = async {
            let mut scope = facts.native_scope(&self.config);
            let write_sink = Arc::new(super::tool_writes::HostWriteSink::new(self.config.clone()));
            let single = if is_write_action(&args) {
                SingleTool::queued(MEMORY_TOOL_NAME, scope, write_sink)
            } else {
                if let Err(error) = super::ops::saas_confinement(
                    &self.config,
                    tinymemory_tools::confine::Policy::Clamp,
                )
                .and_then(|confinement| {
                    confinement
                        .map(|confinement| confinement.reach(scope.reach.clone()))
                        .transpose()
                })
                .map(|reach| {
                    if let Some(reach) = reach {
                        scope.reach = Some(reach);
                    }
                }) {
                    return ToolResult::error(render_error(error));
                }
                let engine = match super::engine::resolve(&self.config).await.engine() {
                    Ok(bound) => bound.engine,
                    Err(error) => return ToolResult::error(render_error(error)),
                };
                SingleTool::new(MEMORY_TOOL_NAME, MemoryTools::with_scope(engine, scope))
                    .with_write_sink(write_sink)
            };
            let call_args = args.clone();
            let thread_window = facts.in_prompt.clone();
            let call = async move {
                single
                    .call_with_details(call_args, CallContext { thread_window })
                    .await
            };
            let outcome = if is_write_action(&args) {
                let write_dispatch = dispatch.clone();
                match write_dispatch.spawn(call).await {
                    Ok(result) => result,
                    Err(_) => {
                        return ToolResult::error(render_error(
                            tinymemory_api::Error::Unavailable(
                                "memory write queue unavailable".into(),
                            ),
                        ));
                    }
                }
            } else {
                call.await
            };
            let outcome = match outcome {
                Ok(outcome) => outcome,
                Err(error) => return ToolResult::error(render_error(error)),
            };
            if let Some(recall) = outcome.recall {
                if let Some(thread_id) = facts.thread_id.as_deref() {
                    crate::web_chat::citations::record_recall_citations(
                        &dispatch,
                        thread_id,
                        &recall.citations,
                    );
                }
                BUS.publish(DomainEvent::MemoryRecalled {
                    query: recall.question,
                    hit_count: recall.citations.len(),
                    thread_id: facts.chat.as_ref().map(|chat| chat.thread_id.clone()),
                    client_id: facts.chat.as_ref().map(|chat| chat.client_id.clone()),
                });
            }
            if is_write_action(&args) {
                super::tool_writes::schedule(&dispatch, self.config.clone(), &facts.reach.at);
            }
            ToolResult::success(outcome.value.to_string())
        };

        if is_write_action(&args) {
            call.await
        } else {
            match self.budget.run(run_id, call).await {
                Ok(result) => result,
                Err(refusal) => budget_refusal(refusal),
            }
        }
    }
}

#[async_trait]
impl Tool for MemoryTool {
    fn name(&self) -> &str {
        MEMORY_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Long-term memory across conversations, documents and learnings. `recall` searches memory for a question and returns what it holds (facts and dated conversation lines) for you to answer from; it does not answer for you. Search matches meaning and wording: if a search misses something, search again with different words before concluding it is not in memory, and search separately for each part of a question that combines several things. `fetch` returns matching items five at a time, best first, each with its id (filter by metadata such as workspace, repo, file_path, kinds). When it also returns `next_cursor`, more items match: for the next five of the same search, call fetch again with the same query and filter and `cursor` set to it. A cursor only continues the search it came from; for different words, fetch without one. `learn` queues a durable fact, preference, procedure or correction about the user or their work, shared with every agent working alongside you; `forget` queues removal of items by id. Write acknowledgements mean locally queued, not yet saved or removed remotely; do not repeat them to force indexing. Cached memory may be added as <memory-context>; use `recall` or `fetch` for current information, and set `refers_to` when the question is about a particular time. Recall before asking the user something they may already have told you; learn things worth remembering next time."
    }

    fn parameters_schema(&self) -> Value {
        tinyagents_harness_tools::static_schema!(include_str!("parameters/memory.json"))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn permission_level_with_args(&self, args: &Value) -> PermissionLevel {
        match permission_for(args) {
            PermissionClass::ReadOnly => PermissionLevel::ReadOnly,
            PermissionClass::Write => PermissionLevel::Write,
        }
    }

    fn is_concurrency_safe(&self, args: &Value) -> bool {
        matches!(concurrency_for(args), ConcurrencyClass::Safe)
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let facts = CallFacts::gather(&self.config, context);
        let dispatch = crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(context);
        let run_id = context
            .and_then(ToolRunContext::host_extension)
            .and_then(|any| any.downcast_ref::<tinyagents_harness::tool::ToolExecutionContext>())
            .map(|ctx| ctx.run_id.as_str().to_string());
        Ok(self
            .call_with_facts(args, dispatch, facts, run_id.as_deref())
            .await)
    }
}

fn render_error(error: tinymemory_api::Error) -> String {
    format!("{} ({})", error, error.code())
}

fn budget_refusal(refusal: Refusal) -> ToolResult {
    match refusal {
        Refusal::Denied(reason) => {
            let message = match reason {
                tinymemory_tools::DenialReason::TrackingFull => {
                    format!("Memory is busy with other runs. {CONTINUE_WITHOUT_MEMORY}")
                }
                _ => format!("Memory budget exhausted for this run. {CONTINUE_WITHOUT_MEMORY}"),
            };
            ToolResult::error(message).with_metadata(json!({ REFUSAL_METADATA_KEY: "refused" }))
        }
        Refusal::TimedOut => ToolResult::error(format!(
            "Memory did not finish within its latency budget and is unavailable for this run. {CONTINUE_WITHOUT_MEMORY}"
        )),
    }
}

/// Whether a call was refused before TinyMemory started its action.
pub fn is_budget_refusal(result: &ToolResult) -> bool {
    result
        .metadata
        .as_ref()
        .is_some_and(|metadata| metadata.get(REFUSAL_METADATA_KEY).is_some())
}

#[cfg(test)]
#[path = "tools_activity_tests.rs"]
mod activity_tests;
#[cfg(test)]
#[path = "tools_agent_tests.rs"]
mod agent_tests;
#[cfg(test)]
#[path = "tools_budget_tests.rs"]
mod budget_tests;
#[cfg(test)]
#[path = "tool_budget_tests.rs"]
mod native_budget_tests;
#[cfg(test)]
#[path = "tools_offline_tests.rs"]
mod offline_tests;
#[cfg(test)]
#[path = "tools_queue_failure_tests.rs"]
mod queue_failure_tests;
#[cfg(test)]
#[path = "tools_queue_tests.rs"]
mod queue_tests;
#[cfg(test)]
#[path = "tools_read_tests.rs"]
mod read_tests;
#[cfg(test)]
#[path = "tools_saas_tests.rs"]
mod saas_tests;
#[cfg(test)]
#[path = "tools_schema_tests.rs"]
mod schema_tests;
#[cfg(test)]
#[path = "tools_test_support.rs"]
mod test_support;
#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
#[cfg(test)]
use test_support::run_action;
