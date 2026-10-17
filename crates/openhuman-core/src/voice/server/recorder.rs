//! Host handle on the module-owned hotkey -> record -> finish state machine.
//!
//! The phase, generation guard and setup / stop-intent buffering live in the
//! `tinyvoice` module (`SessionOpen` / `SessionAdvance`). This wrapper only
//! reports facts to it, mirrors the phase it answers with so `status()` needs
//! no bus call, and serialises the call and the mirror write so two tasks
//! cannot leave the mirror behind the module.

use std::sync::Arc;

use log::{debug, warn};
use tokio::sync::Mutex;

use crate::config::Config;
use crate::modules::voice::{RecorderSession, SessionAdvance, SessionInput};

use super::types::ServerState;
use super::LOG_PREFIX;

/// A recording session plus the phase mirror it keeps current.
#[derive(Clone)]
pub(super) struct Recorder {
    session: RecorderSession,
    state: Arc<Mutex<ServerState>>,
}

impl Recorder {
    pub(super) fn new(session: RecorderSession, state: Arc<Mutex<ServerState>>) -> Self {
        Self { session, state }
    }

    /// Report `input`; `None` when the module could not be reached or the
    /// session is gone. A failure is logged, never fatal: a stale pipeline
    /// finishing after the server stopped lands here by design.
    pub(super) async fn advance(
        &self,
        config: &Config,
        input: SessionInput,
    ) -> Option<SessionAdvance> {
        let mut state = self.state.lock().await;
        match self.session.advance(config, input).await {
            Ok(reply) => {
                debug!(
                    "{LOG_PREFIX} session input={input:?} accepted={} phase={:?} generation={} action={:?}",
                    reply.accepted, reply.phase, reply.generation, reply.action
                );
                *state = reply.phase.into();
                Some(reply)
            }
            Err(error) => {
                warn!("{LOG_PREFIX} session input={input:?} failed: {error}");
                None
            }
        }
    }

    /// Release the module-side session.
    pub(super) async fn close(&self, config: &Config) {
        if let Err(error) = self.session.close(config).await {
            warn!("{LOG_PREFIX} closing the recording session failed: {error}");
        }
    }
}
