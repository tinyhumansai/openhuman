//! Host-routed delivery of a hosted profile's scheduled output.
//!
//! A host that embeds profiles (`profiles::ProfileHost`) owns the conversations
//! its users talk in, so it can route a scheduled reply better than the core
//! can. With a [`CronDelivery`] installed, an `origin`-mode run of a profile's
//! job hands its output to the host instead of writing into a local thread or
//! channel: the host is told which profile, which conversation the job was
//! created in, and the job's recorded origin. A run with nothing to say (blank
//! or `NO_REPLY`) never reaches it.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;

use crate::cron::JobOrigin;

/// One scheduled reply for the host to route.
#[derive(Debug, Clone, PartialEq)]
pub struct CronReply {
    /// The profile whose job produced the reply.
    pub profile_id: String,
    /// The job that ran.
    pub job_id: String,
    /// This run; stable across redelivery of the same reply.
    pub run_id: String,
    /// The thread the job was created in, when it records one.
    pub thread_id: Option<String>,
    /// The conversation snapshot the job was created with.
    pub origin: Option<JobOrigin>,
    /// What the job said.
    pub text: String,
}

/// The host's delivery callback.
#[async_trait]
pub trait CronDelivery: Send + Sync {
    /// Route `reply` to the user. An error is recorded on the run as a failed
    /// delivery.
    ///
    /// # Errors
    ///
    /// The host's reason the reply could not be routed.
    async fn deliver(&self, reply: CronReply) -> Result<(), String>;
}

static SINK: RwLock<Option<Arc<dyn CronDelivery>>> = RwLock::new(None);

/// Install the process's host delivery, returning the one it replaces.
pub fn install(sink: Arc<dyn CronDelivery>) -> Option<Arc<dyn CronDelivery>> {
    SINK.write()
        .unwrap_or_else(|e| e.into_inner())
        .replace(sink)
}

/// Remove the installed delivery. Returns whether there was one.
pub fn clear() -> bool {
    SINK.write()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .is_some()
}

/// The installed delivery, if any.
pub fn installed() -> Option<Arc<dyn CronDelivery>> {
    SINK.read().unwrap_or_else(|e| e.into_inner()).clone()
}
