//! A subject's audit stream, leased by the sandbox planner.
//!
//! The module's `Plan2` and `ConsumeSandboxPlan2` each append one audit event to
//! the subject's stream and require the host's exact correlation (event id,
//! committed tail, policy generation). The approval engine allocates correlations
//! under the subject's stream lock; the planner takes the same lock, so an approval
//! event and a sandbox event of one subject can never claim the same tail.

use std::sync::Arc;

use tinysecurity_bus::{AuditCorrelation2, SubjectScope};

use super::scopes::{audit_entry, ScopeEntry};
use crate::config::Config;

/// The audit stream of one subject.
#[derive(Clone)]
pub(crate) struct AuditStream {
    entry: Arc<ScopeEntry>,
}

impl std::fmt::Debug for AuditStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditStream").finish_non_exhaustive()
    }
}

impl AuditStream {
    /// The persisted stream of `subject`, ready for the module's sink callbacks.
    pub(crate) async fn open(config: &Config, subject: SubjectScope) -> Result<Self, String> {
        Ok(Self {
            entry: audit_entry(config, subject).await?,
        })
    }

    /// The host-persisted stream id the module is told at registration.
    pub(crate) fn stream_id(&self) -> &str {
        &self.entry.stream_id
    }

    /// Hold the stream while a correlation is allocated and its event is appended.
    pub(crate) async fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.entry.stream.lock().await
    }

    /// The correlation of the stream's next event. The caller holds [`Self::lock`],
    /// so the tail read here is the tail the module extends.
    pub(crate) async fn correlation(&self, generation: u64) -> Result<AuditCorrelation2, String> {
        let expected_tail = self.entry.rows.tail(&self.entry.stream_id).await?;
        Ok(AuditCorrelation2 {
            event_id: uuid::Uuid::new_v4().simple().to_string(),
            expected_tail,
            generation,
        })
    }
}

#[cfg(test)]
#[path = "audit_stream_tests.rs"]
mod tests;
