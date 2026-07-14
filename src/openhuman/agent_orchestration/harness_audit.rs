//! Public facade for the live `harness-subagent-audit` debug binary.
//!
//! Keep this surface narrow so the binary can inspect durable async sub-agent
//! state without depending on the full reusable-session or running-registry
//! modules as public API.

use std::path::PathBuf;

use crate::openhuman::agent::harness::run_queue::QueueMode;

use super::{running_subagents, subagent_sessions};

pub use subagent_sessions::{DurableSubagentSession, DurableSubagentStatus};

#[derive(Clone)]
pub struct AuditSubagentSessionStore {
    inner: subagent_sessions::SubagentSessionStore,
}

impl AuditSubagentSessionStore {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self {
            inner: subagent_sessions::SubagentSessionStore::new(workspace_dir),
        }
    }

    pub fn path(&self) -> PathBuf {
        self.inner.path()
    }

    pub fn load(&self) -> Result<Vec<DurableSubagentSession>, String> {
        self.inner.load()
    }
}

/// Why a debug-audit steer could not be delivered.
#[derive(Debug, PartialEq, Eq)]
pub enum AuditSteerError {
    Unknown,
    NotOwned,
    AlreadyDone,
}

impl From<running_subagents::SteerError> for AuditSteerError {
    fn from(error: running_subagents::SteerError) -> Self {
        match error {
            running_subagents::SteerError::Unknown => Self::Unknown,
            running_subagents::SteerError::NotOwned => Self::NotOwned,
            running_subagents::SteerError::AlreadyDone => Self::AlreadyDone,
        }
    }
}

pub async fn steer_running_subagent(
    task_id: &str,
    parent_session: &str,
    message: String,
) -> Result<(), AuditSteerError> {
    running_subagents::steer(task_id, parent_session, message, QueueMode::Steer)
        .await
        .map_err(AuditSteerError::from)
}
