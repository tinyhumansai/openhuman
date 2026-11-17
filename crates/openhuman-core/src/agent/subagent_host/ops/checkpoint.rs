//! OpenHuman billing adapter for TinyAgents' cap-hit checkpoint summary.

use std::sync::{Arc, Mutex};
use tinyagents_harness::summarization::CapToolResult;
use tinyagents_orchestration::subagent::{summarize_at_cap, CapSummaryModel};
use tinyinference_llm::message::Message;
use tinyinference_llm::model::{ChatModel, ModelRequest};
use tinyinference_llm::usage::BilledUsage;

/// The provider usage from a summary call, charged by the host after the owner
/// API applies its shared digest and fallback policy.
pub(super) struct SubagentCheckpointOutcome {
    pub(super) text: String,
    pub(super) usage: Option<BilledUsage>,
}

/// The host owns provider invocation and billing; TinyAgents owns summary
/// selection, deterministic fallback, and the bounded tool-result digest.
pub(super) struct SubagentCheckpoint {
    pub(super) chat_model: Arc<dyn ChatModel<()>>,
    pub(super) agent_id: String,
    pub(super) model: String,
    pub(super) max_output_tokens: u32,
    pub(super) trace: Option<Arc<crate::agent::tinyagents::SupplementalTrace>>,
}

impl SubagentCheckpoint {
    pub(super) async fn summarize_cap_hit(
        &self,
        results: &[CapToolResult<'_>],
        max_iterations: usize,
    ) -> anyhow::Result<SubagentCheckpointOutcome> {
        let model = HostCapSummaryModel {
            checkpoint: self,
            usage: Mutex::new(None),
        };
        let summary = summarize_at_cap(Some(&model), max_iterations, results, 800).await;
        let usage = model
            .usage
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(SubagentCheckpointOutcome {
            text: summary.text,
            usage,
        })
    }
}

struct HostCapSummaryModel<'a> {
    checkpoint: &'a SubagentCheckpoint,
    usage: Mutex<Option<BilledUsage>>,
}

#[async_trait::async_trait]
impl CapSummaryModel for HostCapSummaryModel<'_> {
    async fn summarize(&self, max_calls: usize, digest: &str) -> Result<String, String> {
        let summary_input = vec![Message::user(format!(
            "You are sub-agent `{}` and reached your tool-call limit before finishing. Here are \
             the tool calls you made and their results — compile a brief progress checkpoint (what you \
             accomplished, what still remains) for the agent that delegated to you. Do not call tools.\n\n{digest}",
            self.checkpoint.agent_id,
        ))];
        let request =
            ModelRequest::new(summary_input).with_max_tokens(self.checkpoint.max_output_tokens);
        let trace_input =
            serde_json::to_value(&request.messages).unwrap_or(serde_json::Value::Null);
        let trace_call = self
            .checkpoint
            .trace
            .as_ref()
            .map(|trace| (trace.clone(), trace.begin(&self.checkpoint.model)));
        match self.checkpoint.chat_model.invoke(&(), request).await {
            Ok(mut response) => {
                // Direct ChatModel invocation skips the normal stream adapter's
                // response normalization. Keep the product fold and native trace
                // on the same message-level usage when providers only fill it there.
                if response.usage.is_none() {
                    response.usage = response.message.usage;
                }
                if let Some((trace, (call_id, started_at_ms))) = trace_call {
                    trace.completed(
                        call_id,
                        &self.checkpoint.model,
                        started_at_ms,
                        trace_input,
                        &response,
                    );
                }
                let usage = tinyinference_llm::usage::usage_info_from_response(&response);
                *self
                    .usage
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = usage;
                let (prose, _) = tinytools_agent::parse_tool_calls(&response.text());
                Ok(prose)
            }
            Err(error) => {
                if let Some((trace, (call_id, started_at_ms))) = trace_call {
                    trace.failed(call_id, &self.checkpoint.model, started_at_ms);
                }
                tracing::warn!(
                    agent_id = %self.checkpoint.agent_id,
                    max_calls,
                    error = %error,
                    "[subagent_host] checkpoint summary call failed — using deterministic fallback"
                );
                Err(error.to_string())
            }
        }
    }
}

#[cfg(test)]
#[path = "checkpoint_tests.rs"]
mod tests;
