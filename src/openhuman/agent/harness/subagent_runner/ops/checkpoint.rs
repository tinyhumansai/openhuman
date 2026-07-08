//! Sub-agent cap-hit checkpoint summary.
//!
//! When the iteration cap is hit, summarize the run-so-far into a resumable
//! checkpoint (so the delegating agent can continue from partial progress)
//! instead of erroring. Falls back to a deterministic digest summary if the
//! summarization call fails or returns no prose.

use crate::openhuman::inference::provider::{ChatMessage, ChatRequest, Provider, UsageInfo};

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
pub(super) struct SubagentCheckpoint<'a> {
    pub(super) provider: &'a dyn Provider,
    pub(super) model: String,
    pub(super) temperature: f64,
    pub(super) agent_id: String,
    pub(super) max_output_tokens: u32,
}

impl SubagentCheckpoint<'_> {
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
        let summary_input = vec![ChatMessage::user(format!(
            "You are sub-agent `{agent_id}` and reached your tool-call limit before finishing. Here are \
             the tool calls you made and their results — compile a brief progress checkpoint (what you \
             accomplished, what still remains) for the agent that delegated to you. Do not call tools.\n\n{digest}"
        ))];
        match self
            .provider
            .chat(
                ChatRequest {
                    messages: &summary_input,
                    tools: None,
                    stream: None,
                    // Bounded progress-summary turn; cap also keeps the
                    // reservation-pricing pre-flight realistic (TAURI-RUST-C62).
                    max_tokens: Some(self.max_output_tokens),
                },
                &self.model,
                self.temperature,
            )
            .await
        {
            Ok(resp) => {
                let usage = resp.usage.clone();
                let raw = resp.text.unwrap_or_default();
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
