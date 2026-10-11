use anyhow::Result;
use tinyagents_runtime::{ResumeMode, SessionTurnRequest, TurnOptions};

use crate::agent::{
    message_convert::user_message_from_text, session_host::types::OpenHumanSessionHost,
    tinyagents::host::OpenHumanRunContext,
};

impl OpenHumanSessionHost {
    /// Dispatch a turn with authority supplied by its owning entry point.
    pub async fn turn_with_origin(
        &mut self,
        user_message: &str,
        origin: Option<&crate::agent::turn_origin::AgentTurnOrigin>,
    ) -> Result<String> {
        self.ensure_runtime_session()?;
        let staged = super::attachment_input::stage_turn(self, user_message, origin).await?;
        let mut context = OpenHumanRunContext::new();
        context.origin = origin.cloned();
        context.progress = self.on_progress.clone();
        context.thread_id = self.thread_id.clone();
        context.workspace = self.workspace_descriptor.clone();
        // The web backstop's deadline, when this turn runs under one: the
        // harness winds down before it instead of being dropped by it.
        context.turn_deadline = crate::agent::turn_deadline::current();
        let cancellation = context.cancellation.clone();
        let root_config = context.root_run_config("openhuman-session");
        let mut options = TurnOptions {
            request_id: origin.and_then(|origin| match origin {
                crate::agent::turn_origin::AgentTurnOrigin::WebChat { request_id, .. } => {
                    request_id.clone()
                }
                _ => None,
            }),
            thread_id: self.thread_id.clone(),
            stream: self.on_progress.is_some(),
            session: self.session.clone(),
            resume: self.turn_resume_mode(),
            cancellation,
            run_context: context.into_tinyagents(root_config),
        };
        // Load a bound, otherwise empty session before its normal lifecycle so
        // the host prelude can rebuild its recorded integration executors.
        if matches!(options.resume, ResumeMode::Session)
            && self
                .runtime_session
                .as_ref()
                .is_some_and(|session| session.history().is_empty())
        {
            let runtime = self
                .runtime_session
                .as_mut()
                .expect("runtime session initialized");
            let resumed = runtime
                .resume(&options)
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            if resumed.loaded {
                let recorded_tools = runtime.recorded_tools().cloned();
                if let Some(prelude) = self
                    .runtime_state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .prelude
                    .clone()
                {
                    prelude.adopt_recorded_tools(recorded_tools.as_ref());
                }
            }
        }
        // Persisted declarations are historical facts, while the hook still
        // supplies the only executable sources and current policy.
        options.run_context.data.recorded_tool_names = std::sync::Arc::new(
            self.runtime_session
                .as_ref()
                .and_then(|runtime| runtime.recorded_tools())
                .map(|snapshot| {
                    snapshot
                        .specs()
                        .iter()
                        .map(|spec| spec.name.clone())
                        .collect()
                })
                .unwrap_or_default(),
        );
        let agent_id = self.agent_definition_id.clone();
        let turn = self
            .runtime_session
            .as_mut()
            .expect("runtime session initialized")
            .turn(
                SessionTurnRequest::new(user_message_from_text(staged.as_str())),
                options,
            );
        let outcome = crate::memory::scope::within_agent(&agent_id, Box::pin(turn))
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        Ok(outcome.output.unwrap_or_default())
    }
}

/// Applies the pending one-turn overrides to this turn's resume mode.
pub(in crate::agent::session_host) fn begin_turn_resume(
    state: &mut super::OpenHumanSessionState,
    resume: &mut ResumeMode,
) {
    let overrides = std::mem::take(&mut state.pending_turn_overrides);
    if overrides.suppress_transcript_autoload {
        *resume = ResumeMode::Never;
    }
    state.active_turn_overrides = overrides;
}
