//! Sub-agent cap-hit checkpoint summary.
//!
//! When the iteration cap is hit, summarize the run-so-far into a resumable
//! checkpoint (so the delegating agent can continue from partial progress)
//! instead of erroring. Falls back to a deterministic digest summary if the
//! summarization call fails or returns no prose.

use crate::openhuman::inference::provider::UsageInfo;
use std::sync::Arc;
use tinyagents::harness::message::Message;
use tinyagents::harness::model::{ChatModel, ModelRequest};

/// A checkpoint result. `usage`, when present, is the provider usage from the
/// summary call so the caller can fold it into sub-agent token/cost accounting.
pub(super) struct SubagentCheckpointOutcome {
    pub(super) text: String,
    pub(super) usage: Option<UsageInfo>,
}

/// Sub-agent cap-hit summary: when the iteration cap is hit, summarize the
/// run-so-far into a resumable checkpoint (so the delegating agent can continue
/// from partial progress) instead of erroring. Falls back to a deterministic
/// digest summary if the summarization call fails or returns no prose.
///
/// The summary runs on a crate [`ChatModel`] (built from the turn's
/// [`TurnModelSource`](crate::openhuman::tinyagents::TurnModelSource) — model +
/// temperature baked in), so the checkpoint no longer names the `Provider` trait
/// (issue #4249, Phase 3 / Motion A).
pub(super) struct SubagentCheckpoint {
    pub(super) chat_model: Arc<dyn ChatModel<()>>,
    pub(super) agent_id: String,
    pub(super) max_output_tokens: u32,
}

impl SubagentCheckpoint {
    pub(super) async fn summarize_cap_hit(
        &self,
        digest: &str,
        max_iterations: usize,
    ) -> anyhow::Result<SubagentCheckpointOutcome> {
        let agent_id = &self.agent_id;
        let deterministic = format!(
            "I reached my tool-call limit ({max_iterations} steps) before finishing this task. \
             Progress so far (tool calls + results):\n{digest}\n\nThe task is incomplete — the above is \
             what I accomplished; continue from here."
        );
        let summary_input = vec![Message::user(format!(
            "You are sub-agent `{agent_id}` and reached your tool-call limit before finishing. Here are \
             the tool calls you made and their results — compile a brief progress checkpoint (what you \
             accomplished, what still remains) for the agent that delegated to you. Do not call tools.\n\n{digest}"
        ))];
        // Bounded progress-summary turn; the cap also keeps the reservation-pricing
        // pre-flight realistic (TAURI-RUST-C62). Temperature is baked into the model.
        let request = ModelRequest::new(summary_input).with_max_tokens(self.max_output_tokens);
        match self.chat_model.invoke(&(), request).await {
            Ok(resp) => {
                let usage = crate::openhuman::tinyagents::model::usage_info_from_response(&resp);
                let raw = resp.text();
                let (prose, _) = super::super::super::parse::parse_tool_calls(&raw);
                let text = if prose.trim().is_empty() {
                    deterministic
                } else {
                    prose
                };
                Ok(SubagentCheckpointOutcome { text, usage })
            }
            Err(e) => {
                tracing::warn!(
                    agent_id = %self.agent_id,
                    error = %e,
                    "[subagent_runner] checkpoint summary call failed — using deterministic fallback"
                );
                Ok(SubagentCheckpointOutcome {
                    text: deterministic,
                    usage: None,
                })
            }
        }
    }
}
