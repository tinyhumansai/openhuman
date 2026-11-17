//! Memory's event subscriber.
//!
//! `memory::system_jobs` runs memory's cron jobs
//! ([`DomainEvent::CronSystemJobDue`]):
//!
//! - `memory_sources_sync` starts the due source syncs;
//! - `memory_background` runs the queued belief builds and deferred ingests
//!   (`lifecycle::jobs`), after resuming a v1 import the app quit in the
//!   middle of (`import::resume_interrupted`).
//!
//! `memory::pending_deletions` drains the deletions queued while memory was
//! off ([`super::deletion`]) when a credential is stored
//! ([`DomainEvent::CredentialChanged`], any kind but `cleared`): the next
//! sign-in after a disconnect or a thread delete finishes the delete. The
//! background job drains them too, so a deletion that failed is retried.
//!
//! Turns are not ingested from the bus: the session host calls the lifecycle
//! hooks itself, under the session's own config (`lifecycle::hooks`).

use crate::core::runtime::DispatchCtx;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use chrono::Utc;
use tinybus::{EventHandler, SubscriptionHandle};

use crate::core::events::DomainEvent;

use super::lifecycle::jobs::BACKGROUND_JOB;

/// Cron job that starts due source syncs.
pub const SOURCES_SYNC_JOB: &str = "memory_sources_sync";

/// The retired `context.md` refresh job; `cron::system_jobs` removes its row.
pub const RETIRED_CONTEXT_REFRESH_JOB: &str = "memory_context_refresh";

static JOBS_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

static DELETIONS_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Drains the pending deletions once a credential is stored.
pub(crate) struct PendingDeletionsSubscriber;

#[async_trait]
impl EventHandler<DomainEvent> for PendingDeletionsSubscriber {
    fn name(&self) -> &str {
        "memory::pending_deletions"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["auth"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::CredentialChanged { kind, tenant } = event else {
            return;
        };
        if kind == "cleared" {
            return;
        }
        let Some(ctx) = DispatchCtx::for_event(tenant.as_ref()) else {
            tracing::debug!("[memory:bus] credential change has no tenant; deletions wait");
            return;
        };
        let config = match ctx.load_config().await {
            Ok(config) => config,
            Err(error) => {
                tracing::debug!(error = %error, "[memory:bus] config unavailable; deletions wait");
                return;
            }
        };
        drain_pending_deletions(&ctx, &config, kind).await;
    }
}

/// Runs the deletions queued while memory was off, after a credential of
/// `kind` was stored.
pub(crate) async fn drain_pending_deletions(
    ctx: &DispatchCtx,
    config: &crate::config::Config,
    kind: &str,
) -> usize {
    super::tool_writes::schedule_all(ctx, Arc::new(config.clone()));
    let settled = super::deletion::drain(config).await;
    tracing::debug!(kind = %kind, settled, "[memory:bus] pending deletions drained after sign-in");
    settled
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
        let DomainEvent::CronSystemJobDue { job, tenant } = event else {
            return;
        };
        if job != SOURCES_SYNC_JOB && job != BACKGROUND_JOB {
            return;
        }
        let Some(ctx) = DispatchCtx::for_event(tenant.as_ref()) else {
            tracing::debug!(job = %job, "[memory:bus] system job has no tenant; skipped");
            return;
        };
        let config = match ctx.load_config().await {
            Ok(config) => config,
            Err(error) => {
                tracing::debug!(error = %error, job = %job, "[memory:bus] config unavailable");
                return;
            }
        };
        run_system_job(&ctx, &config, job).await;
        if job == SOURCES_SYNC_JOB {
            sync_live_agent_sources().await;
        }
    }
}

/// Starts the due source syncs of every live embedded agent, each under its
/// own context so its memory binding and sources apply.
async fn sync_live_agent_sources() {
    for (agent_id, ctx) in crate::core::runtime::AgentContextRegistry::live() {
        let dispatch = crate::core::runtime::DispatchCtx::new(ctx.clone());
        crate::core::runtime::CoreContext::scope(ctx, async {
            match crate::config::ops::load_current_or_init(&dispatch).await {
                Ok(config) => {
                    tracing::debug!(agent = %agent_id, "[memory:bus] agent source sync");
                    run_system_job(&dispatch, &config, SOURCES_SYNC_JOB).await;
                }
                Err(error) => {
                    tracing::debug!(agent = %agent_id, %error, "[memory:bus] agent config unavailable");
                }
            }
        })
        .await;
    }
}

/// Runs one memory cron job against `config`.
pub async fn run_system_job(ctx: &DispatchCtx, config: &crate::config::Config, job: &str) {
    match job {
        SOURCES_SYNC_JOB => {
            let started = super::sources::sync_due(ctx, config, Utc::now()).await;
            tracing::debug!(
                started = started.len(),
                "[memory:bus] due source syncs started"
            );
        }
        BACKGROUND_JOB => {
            super::tool_writes::schedule_all(ctx, Arc::new(config.clone()));
            super::import::resume_interrupted(ctx, config).await;
            super::layout_migration::tick(
                ctx,
                config,
                std::sync::Arc::new(super::layout_migration::AppHost::new(config.clone())),
                std::sync::Arc::new(super::import::scheduler_paused),
            );
            super::lifecycle::jobs::run_due(config).await;
            super::deletion::drain_due(config).await;
        }
        _ => {}
    }
}

/// Registers memory's subscriber. Idempotent.
pub fn register_memory_subscribers() {
    if JOBS_HANDLE.get().is_none() {
        match crate::core::bus::BUS.subscribe(Arc::new(SystemJobsSubscriber)) {
            Some(handle) => {
                let _ = JOBS_HANDLE.set(handle);
                tracing::info!("[memory:bus] memory subscribers registered");
            }
            None => tracing::warn!("[memory:bus] system jobs not registered: no bus"),
        }
    }
    if DELETIONS_HANDLE.get().is_none() {
        match crate::core::bus::BUS.subscribe(Arc::new(PendingDeletionsSubscriber)) {
            Some(handle) => {
                let _ = DELETIONS_HANDLE.set(handle);
            }
            None => tracing::warn!("[memory:bus] pending deletions not registered: no bus"),
        }
    }
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;
