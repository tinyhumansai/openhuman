//! The model behind TinyJuice's tool-output summary.
//!
//! When a tool returns a huge payload — a Composio action dumping 200 KB of
//! JSON, a long web page, a multi-thousand-line log — TinyJuice's summary
//! stage decides whether it is worth summarizing, writes the extraction
//! prompt (including the caller's `summary_focus`), caches and breaker-guards
//! the result, and keeps the original recoverable. What it cannot do is call a
//! model: that is the host's, and it has to run under the turn that made the
//! tool call so it inherits the turn's model, cancellation, thread and
//! lineage.
//!
//! [`DynPayloadSummarizer`] is that seam. `ToolOutputMiddleware` asks it to
//! [`prepare`](tinyagents_harness::summarization::payload_summarizer::PayloadSummarizer::prepare) a call bound to the current turn,
//! registers it with [`crate::inference::tokenjuice::generate`], and sends the
//! ticket's token with the result. TinyJuice calls back through
//! `MlHost.Generate` only if it decides to summarize.
//!
//! Only the orchestrator session gets one
//! ([`crate::agent::session_host::builder::AgentBuilder`] checks
//! `agent_id == "orchestrator"`); every other agent's results are compacted
//! without a summary.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use tinyagents_harness::context::RunContext;
use tinyagents_harness::summarization::payload_summarizer::{
    run_unary_summary, summary_child_config, PayloadSummarizer as HarnessPayloadSummarizer,
};
use tracing::debug;

use crate::agent::harness::host_definition::AgentDefinition;
use crate::agent::subagent_host;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use {
    crate::inference::tokenjuice::generate::PreparedGenerate, tinyjuice_bus::wire::GenerateRequest,
};

/// The host's payload-summary seam: the harness contract
/// ([`HarnessPayloadSummarizer`]) bound to OpenHuman's turn context, the
/// prepared call TinyJuice drives and `anyhow` errors.
///
/// Public because it is an embedder seam: the default implementation runs the
/// `summarizer` agent definition's model, and a host that routes models its own
/// way supplies its own. The prompt is not part of this — TinyJuice writes it
/// and sends it in the [`GenerateRequest`].
pub type DynPayloadSummarizer = dyn HarnessPayloadSummarizer<
    OpenHumanRunContext,
    Prepared = PreparedGenerate,
    Error = anyhow::Error,
>;

/// Runs the `summarizer` agent definition's model as a unary child of the turn.
pub struct SubagentPayloadSummarizer {
    definition: AgentDefinition,
}

impl SubagentPayloadSummarizer {
    /// `definition` supplies the model hint, temperature and iteration limits.
    pub fn new(definition: AgentDefinition) -> Self {
        Self { definition }
    }
}

/// The summary's child context, derived without rebuilding any of the parent's
/// live capabilities. This pairs TinyAgents' canonical `RunContext::child` with
/// OpenHuman's host-state `child` rule, so the call keeps the parent's
/// cancellation, workspace, stores, events, thread and lineage while its route
/// and usage observations stay its own.
pub(crate) fn unary_child_context(
    parent_ctx: &RunContext<OpenHumanRunContext>,
    agent_id: &str,
    max_iterations: usize,
    max_output_tokens: u32,
) -> tinyagents_harness::Result<RunContext<OpenHumanRunContext>> {
    let child_config =
        summary_child_config(parent_ctx, agent_id, max_iterations, max_output_tokens);
    let child_data = parent_ctx
        .data
        .child()
        .with_cancellation(parent_ctx.cancellation.clone());
    let mut child = parent_ctx.child(child_config, child_data)?;
    // TinyAgents links the child's token to the parent's; point the host
    // carrier at that same linked token so host tools observe the child scope
    // (parent cancel cascades down, child cancel leaves the parent running).
    child.data.cancellation = child.cancellation.clone();
    Ok(child)
}

impl HarnessPayloadSummarizer<OpenHumanRunContext> for SubagentPayloadSummarizer {
    type Prepared = PreparedGenerate;
    type Error = anyhow::Error;

    fn prepare(&self, parent_ctx: &RunContext<OpenHumanRunContext>) -> Result<PreparedGenerate> {
        let parent =
            parent_ctx.data.parent.clone().ok_or_else(|| {
                anyhow!("payload summarizer needs the turn's ParentExecutionContext")
            })?;
        let definition = self.definition.clone();
        let thread_id = parent_ctx.data.thread_id.clone();
        let max_output_tokens = definition
            .max_turn_output_tokens
            .unwrap_or(crate::inference::provider::AGENT_TURN_MAX_OUTPUT_TOKENS);
        let child_context = unary_child_context(
            parent_ctx,
            &definition.id,
            definition.max_iterations,
            max_output_tokens,
        )?;
        debug!(
            agent = %definition.id,
            parent_depth = parent_ctx.depth(),
            "[payload_summarizer] prepared a summary call under the parent turn"
        );

        let dispatch = parent_ctx.data.dispatch_ctx();
        Ok(Box::new(move |request: GenerateRequest| {
            let dispatch = dispatch.clone();
            Box::pin(async move {
                let config_loaded = crate::config::ops::load_current_or_init(&dispatch).await;
                let (source, model) = subagent_host::resolve_subagent_source(
                    &definition.model,
                    &definition.id,
                    config_loaded.as_ref().ok(),
                    parent.turn_model_source.clone(),
                    parent.model_name.clone(),
                    false,
                    None,
                    definition.temperature,
                );
                let max_output_tokens = request.max_output_tokens.min(max_output_tokens).max(1);
                let system_prompt =
                    subagent_host::append_subagent_role_contract(request.system, &definition.id);

                let provider_model = tinyinference_llm::model::MaxTokensModel::new(
                    source.build_summarizer(
                        &model,
                        definition.temperature,
                        thread_id.as_deref(),
                    )?,
                    max_output_tokens,
                );
                let provider_model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
                    Arc::new(provider_model);
                let provider_model = match &child_context.data.model_budget {
                    Some(budget) => budget.wrap(provider_model),
                    None => provider_model,
                };
                // Unary, never streaming: a streaming child would publish the
                // internal summary to the user as parent `TextDelta`s.
                Ok(run_unary_summary(
                    child_context,
                    &model,
                    provider_model,
                    definition.max_iterations,
                    system_prompt,
                    request.prompt,
                )
                .await?)
            })
        }))
    }
}

#[cfg(test)]
#[path = "payload_summarizer_tests.rs"]
mod tests;
