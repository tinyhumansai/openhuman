//! The host's session surface: start one, read it back, list its generations.
//!
//! OpenHuman does not own session mechanics. Identity, resume, transcript
//! binding, compaction generations and persistence all live in
//! `tinyagents-session` / `tinyagents-runtime`. What remains here is the small
//! product-facing surface over them — starting a session (binding a thread's
//! identity, [`super::runtime::accessors`]), reading one back, and listing the
//! generations a long conversation has accumulated.

use anyhow::Result;
use tinyagents_runtime::{ResumeMode, TurnOptions};

use crate::agent::tinyagents::host::OpenHumanRunContext;

use super::OpenHumanSessionHost;

impl OpenHumanSessionHost {
    /// Load this session's durable history now, before any turn runs.
    ///
    /// A turn resumes on its own, so this exists for hosts that want the
    /// history in hand first — checking what a thread remembers, or rendering
    /// it — and for tests that assert resume without driving a provider.
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
            session: Some(session),
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
