//! [`Agent::compact_thread`]: compact a stored conversation while its user is
//! idle, so the next turn resends a summary instead of the full history.

use std::sync::Arc;

use super::{Agent, AgentError};
use crate::CoreError;

pub use openhuman_core::agent::{CompactOptions, CompactOutcome};

/// The method name a compaction's errors carry.
const COMPACT_THREAD: &str = "agent.compact_thread";

impl Agent {
    /// Compact `thread_id`'s stored conversation now, with no model turn.
    ///
    /// The thread is resolved exactly as [`Turn::session`](crate::Turn::session)
    /// resolves it, and its head transcript generation is read. Below
    /// [`CompactOptions::min_tokens`] (the compression middleware's own
    /// estimate) nothing is written and [`CompactOutcome::Skipped`] reports
    /// the size. Otherwise the agent's compaction runs as a turn's would
    /// (`[context] compaction_strategy`, `summarizer_model`,
    /// `summarizer_tool_rendering`, `summarizer_max_output_tokens`): the head
    /// generation is sealed unchanged and the next one opens with the
    /// system prompt, the summary as a user-role message, and the kept tail.
    /// The system prompt and recorded tool declarations carry over
    /// byte-identical, so the provider's prefix cache survives.
    ///
    /// A compaction holds the session's turn lock from read to write: a turn
    /// sent on the same thread meanwhile waits for it, then resumes from the
    /// compacted head.
    ///
    /// # Errors
    ///
    /// [`AgentError::Call`] when the agent was removed, inference is not in
    /// this runtime's domains, the thread id is blank, or the compaction
    /// fails (stores, summarizer models, or the transcript write).
    pub async fn compact_thread(
        &self,
        thread_id: &str,
        options: CompactOptions,
    ) -> Result<CompactOutcome, AgentError> {
        let inner = Arc::clone(&self.inner);
        if !inner.ctx.domains().inference {
            return Err(CoreError::Unavailable {
                method: COMPACT_THREAD,
            }
            .into());
        }
        // The provider route and model a turn on this agent runs with.
        let turn = self.turn(String::new());
        let request = turn.request();
        let route = crate::turn::inference_route(request);
        let model_override = request.model_override.clone();
        let temperature = request.temperature;
        let ctx = inner.ctx.clone();
        let runtime = inner.runtime.clone();
        let thread_id = thread_id.to_owned();
        let agent = Arc::clone(&inner);
        let compaction = Box::pin(async move {
            use openhuman_core::inference::host_runtime::ops::AgentChatTarget;
            let host = agent.composed_host_tools();
            let target = AgentChatTarget::Definition {
                definition: &agent.definition,
                host: host.as_ref(),
                seed: None,
                usage: None,
                host_only: agent.host_only,
                untrusted_input: false,
                shape: None,
                observer: None,
                attachments: &[],
            };
            let mut config = agent.config.clone();
            openhuman_core::agent_runs::agent_chat::compact_thread_for(
                &mut config,
                target,
                &thread_id,
                model_override,
                temperature,
                route,
                options,
            )
            .await
            .map_err(|error| CoreError::Rpc {
                method: COMPACT_THREAD,
                message: error.to_string(),
            })
        });
        inner
            .lifecycle
            .admit(&inner.id, COMPACT_THREAD, runtime.run_in(ctx, compaction))
            .await
            .map_err(Into::into)
    }
}
