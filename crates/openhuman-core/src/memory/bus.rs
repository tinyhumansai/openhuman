//! Memory's event subscribers and background flusher.
//!
//! - `memory::conversation_ingest` buffers every
//!   [`DomainEvent::ConversationTurnCommitted`] for conversation ingestion.
//! - `memory::system_jobs` runs the `memory_context_refresh` and
//!   `memory_sources_sync` cron jobs ([`DomainEvent::CronSystemJobDue`]).
//! - A background task stores threads that went idle (`idle_secs`).

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use tinybus::{EventHandler, SubscriptionHandle};
use tinymemory::ToolCallRef;

use crate::core::events::DomainEvent;

use super::conversations::buffer::CommittedTurn;

/// Cron job that recompiles `context.md`.
pub const CONTEXT_REFRESH_JOB: &str = "memory_context_refresh";

/// Cron job that starts due source syncs.
pub const SOURCES_SYNC_JOB: &str = "memory_sources_sync";

/// How often the idle flusher looks for quiet threads.
const IDLE_SWEEP_INTERVAL: Duration = Duration::from_secs(15);

static INGEST_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();
static JOBS_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();
static FLUSHER_STARTED: OnceLock<()> = OnceLock::new();

/// The committed turn an event describes, or `None` for any other event.
#[must_use]
pub fn committed_turn(event: &DomainEvent) -> Option<CommittedTurn> {
    let DomainEvent::ConversationTurnCommitted {
        thread_id,
        agent_id,
        workspace,
        channel,
        user_text,
        assistant_text,
        tool_calls,
        ..
    } = event
    else {
        return None;
    };
    Some(CommittedTurn {
        thread_id: thread_id.clone(),
        agent_id: agent_id.clone(),
        workspace: workspace.clone(),
        channel: channel.clone(),
        user: user_text.clone(),
        assistant: assistant_text.clone(),
        tool_calls: tool_calls
            .iter()
            .map(|call| ToolCallRef {
                name: call.name.clone(),
                id: call.id.clone(),
            })
            .collect(),
        at: Utc::now(),
    })
}

struct ConversationIngestSubscriber;

#[async_trait]
impl EventHandler<DomainEvent> for ConversationIngestSubscriber {
    fn name(&self) -> &str {
        "memory::conversation_ingest"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["agent"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let Some(turn) = committed_turn(event) else {
            return;
        };
        let DomainEvent::ConversationTurnCommitted { workspace_dir, .. } = event else {
            return;
        };
        match crate::config::rpc::load_config_for_workspace_with_timeout(workspace_dir).await {
            Ok(config) => super::conversations::record_turn(&config, turn).await,
            Err(error) => {
                tracing::debug!(error = %error, "[memory:bus] config unavailable; turn dropped")
            }
        }
    }
}

struct SystemJobsSubscriber;

#[async_trait]
impl EventHandler<DomainEvent> for SystemJobsSubscriber {
    fn name(&self) -> &str {
        "memory::system_jobs"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["cron"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::CronSystemJobDue { job } = event else {
            return;
        };
        if job != CONTEXT_REFRESH_JOB && job != SOURCES_SYNC_JOB {
            return;
        }
        let config = match crate::config::rpc::load_config_with_timeout().await {
            Ok(config) => config,
            Err(error) => {
                tracing::debug!(error = %error, job = %job, "[memory:bus] config unavailable");
                return;
            }
        };
        run_system_job(&config, job).await;
    }
}

/// Runs one memory cron job against `config`.
pub async fn run_system_job(config: &crate::config::Config, job: &str) {
    match job {
        CONTEXT_REFRESH_JOB => {
            if !config.memory.context.enabled {
                tracing::debug!("[memory:bus] context disabled; refresh skipped");
                return;
            }
            if let Err(error) = super::context::refresh(config).await {
                tracing::debug!(code = error.code(), "[memory:bus] context refresh skipped");
            }
        }
        SOURCES_SYNC_JOB => {
            let started = super::sources::sync_due(config, Utc::now());
            tracing::debug!(
                started = started.len(),
                "[memory:bus] due source syncs started"
            );
        }
        _ => {}
    }
}

/// Registers memory's subscribers and starts the idle flusher. Idempotent.
pub fn register_memory_subscribers() {
    if INGEST_HANDLE.get().is_none() {
        match crate::core::bus::BUS.subscribe(Arc::new(ConversationIngestSubscriber)) {
            Some(handle) => {
                let _ = INGEST_HANDLE.set(handle);
            }
            None => tracing::warn!("[memory:bus] conversation ingest not registered: no bus"),
        }
    }
    if JOBS_HANDLE.get().is_none() {
        match crate::core::bus::BUS.subscribe(Arc::new(SystemJobsSubscriber)) {
            Some(handle) => {
                let _ = JOBS_HANDLE.set(handle);
            }
            None => tracing::warn!("[memory:bus] system jobs not registered: no bus"),
        }
    }
    if FLUSHER_STARTED.set(()).is_ok() {
        tokio::spawn(async {
            let mut ticker = tokio::time::interval(IDLE_SWEEP_INTERVAL);
            loop {
                ticker.tick().await;
                if let Ok(config) = crate::config::rpc::load_config_with_timeout().await {
                    let flushed = super::conversations::flush_idle(&config, Utc::now()).await;
                    if flushed > 0 {
                        tracing::debug!(flushed, "[memory:bus] idle threads stored");
                    }
                }
            }
        });
        tracing::info!("[memory:bus] memory subscribers registered");
    }
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;
