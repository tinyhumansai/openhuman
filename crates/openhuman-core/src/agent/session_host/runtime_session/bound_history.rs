//! Bound-session history adoption for trusted inbound hosts.
//!
//! Native channels and the relay import an existing conversation into the
//! durable TinyAgents session before its first host turn, and reset it on an
//! explicit channel command. Both act on the exact bound `SessionRef` only;
//! the runtime owns the session identity and reset serialization.

use anyhow::Result;
use tinyagents_runtime::{ResumeMode, TurnOptions};
use tinyagents_session::transcript::TranscriptMessage;

use super::super::types::OpenHumanSessionHost;
use crate::agent::tinyagents::host::OpenHumanRunContext;

impl OpenHumanSessionHost {
    /// Whether this exact bound session already carries authoritative durable
    /// state. A freshly opened header alone is not enough: hosts may still need
    /// to adopt legacy history once. Explicit reset boundaries and recorded
    /// prompt/tool snapshots are authoritative even when the current head is
    /// intentionally empty.
    pub(crate) fn bound_history_is_authoritative(&self) -> Result<bool> {
        let Some(session) = self.session.as_ref() else {
            return Ok(false);
        };
        let locator = self.subagent_transcript_locator();
        let head = locator.head_generation(session);
        let Some(read) = locator.read_session_transcript(&head) else {
            return Ok(false);
        };
        let Some(transcript) = read
            .read_session()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?
        else {
            return Ok(false);
        };
        Ok(!transcript.messages.is_empty()
            || transcript.meta.turn_count > 0
            || transcript.meta.prefix_reset
            || transcript.meta.prefix_message_count.is_some()
            || transcript.tools.is_some())
    }

    /// Exact durable identity used by the method above. Exposed only to the
    /// adjacent session-host tests and callers that need to seed that same
    /// recorded session store.
    #[cfg(test)]
    pub(crate) fn bound_session_ref(&self) -> Option<&tinyagents_session::transcript::SessionRef> {
        self.session.as_ref()
    }

    /// Load legacy history only when the exact bound session has no durable
    /// turn or explicit empty/reset boundary. A failed load returns before the
    /// caller can start a turn and create a misleading empty session.
    pub(crate) async fn adopt_legacy_history_if_needed<F, Fut>(&self, load: F) -> Result<bool>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Vec<TranscriptMessage>>>,
    {
        if self.bound_history_is_authoritative()? {
            tracing::debug!(
                thread_id = ?self.thread_id,
                "[session] bound history already authoritative; legacy import skipped"
            );
            return Ok(false);
        }
        let messages = load().await?;
        self.adopt_imported_history(&messages)
    }

    /// Reset the bound durable transcript without deleting its sealed
    /// generations. The runtime owns the session identity and reset
    /// serialization; this host method only ensures the bound session is
    /// resumed before delegating to it.
    pub(crate) async fn reset_bound_history(&mut self) -> Result<bool> {
        self.ensure_runtime_session()?;
        self.resume_bound_session().await?;
        let reset = self
            .runtime_session
            .as_mut()
            .expect("runtime session initialized")
            .reset_history()
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        tracing::info!(thread_id = ?self.thread_id, reset, "[session] bound history reset");
        Ok(reset)
    }

    /// Install imported conversation rows once, before the first turn binds
    /// this durable session. Empty imports do not reserve a session, and a
    /// transcript already written by a prior turn always wins unchanged.
    pub(crate) fn adopt_imported_history(&self, messages: &[TranscriptMessage]) -> Result<bool> {
        if messages.is_empty() {
            return Ok(false);
        }
        let Some(session) = self.session.as_ref() else {
            return Ok(false);
        };
        let adopted = self
            .subagent_transcript_locator()
            .adopt_messages_if_absent(session, self.runtime_transcript_meta(), messages)?;
        tracing::debug!(
            thread_id = ?self.thread_id,
            imported = messages.len(),
            adopted,
            "[session] adopt imported history"
        );
        Ok(adopted)
    }

    /// Load this session's durable history now, before any turn runs.
    ///
    /// A turn resumes on its own, so this exists for hosts that want the
    /// history in hand first — checking what a thread remembers, resetting it,
    /// or rendering it — and for tests that assert resume without driving a
    /// provider.
    ///
    /// Returns whether history was loaded. `false` means the session has no
    /// durable identity bound (no thread), has nothing persisted yet, or has
    /// already run a turn.
    pub async fn resume_bound_session(&mut self) -> Result<bool> {
        let Some(session) = self.session.clone() else {
            return Ok(false);
        };
        self.ensure_runtime_session()?;

        let mut context = OpenHumanRunContext::new();
        context.thread_id = self.thread_id.clone();
        context.workspace = self.workspace_descriptor.clone();
        let cancellation = context.cancellation.clone();
        let root_config = context.root_run_config("openhuman-session-resume");
        let options = TurnOptions {
            request_id: None,
            thread_id: self.thread_id.clone(),
            stream: false,
            session: self.session_history_locator.as_ref().map(|_| session),
            resume: ResumeMode::Session,
            cancellation,
            run_context: context.into_tinyagents(root_config),
            controls: Default::default(),
        };
        let resumed = self
            .runtime_session
            .as_mut()
            .expect("runtime session initialized")
            .resume(&options)
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        Ok(resumed.loaded)
    }
}
