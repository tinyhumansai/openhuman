//! Event-bus integration for the `task_sources` domain.
//!
//! Subscribes to `ComposioConnectionCreated`: when a user connects a
//! toolkit that has matching enabled task sources, fire a one-shot
//! `ConnectionCreated` fetch so freshly-connected work shows up without
//! waiting for the next periodic tick.

use crate::core::runtime::DispatchCtx;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use tinybus::EventHandler;
use tinybus::SubscriptionHandle;

use super::types::{FetchReason, ProviderSlug};
use super::{pipeline, store};

static CONNECTION_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Fires a one-shot fetch for matching sources when a Composio
/// connection is created.
pub struct TaskSourcesConnectionSubscriber;

#[async_trait]
impl EventHandler<DomainEvent> for TaskSourcesConnectionSubscriber {
    fn name(&self) -> &str {
        "task_sources::connection"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["composio"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::ComposioConnectionCreated {
            toolkit,
            connection_id,
            tenant,
            ..
        } = event
        else {
            return;
        };
        let Some(ctx) = DispatchCtx::for_event(tenant.as_ref()) else {
            tracing::debug!(toolkit = %toolkit, "[task_sources:bus] connection_created has no tenant; skipped");
            return;
        };

        // Only act for toolkits we model as task sources.
        let Ok(provider) = ProviderSlug::parse(toolkit) else {
            return;
        };

        // Every scope may hold sources for this toolkit (`crate::storage`):
        // handle each scope as its own agent, with that agent's config.
        crate::storage::agents::for_each_live_scope(&ctx, "task_sources connection", |scope_ctx| async move {
            fire_in_scope(&scope_ctx, provider, toolkit, connection_id).await
        })
        .await;
    }
}

/// [`fire_for_connection`] for the current scope, with the configuration
/// loaded inside it — so an agent whose own config disables task sources is
/// skipped even when the process config enables them, and the other way
/// round.
async fn fire_in_scope(
    ctx: &DispatchCtx,
    provider: ProviderSlug,
    toolkit: &str,
    connection_id: &str,
) {
    let config = match ctx.load_config().await {
        Ok(config) => config,
        Err(e) => {
            tracing::debug!(error = %e, "[task_sources:bus] load_config failed, skipping scope");
            return;
        }
    };
    if !config.task_sources.enabled {
        return;
    }
    fire_for_connection(ctx, &config, provider, toolkit, connection_id).await;
}

/// Starts a one-shot fetch for every enabled source of `provider` in the
/// current storage scope that the new connection serves.
async fn fire_for_connection(
    ctx: &DispatchCtx,
    config: &crate::config::Config,
    provider: ProviderSlug,
    toolkit: &str,
    connection_id: &str,
) {
    let sources = match store::list_sources(config) {
        Ok(sources) => sources,
        Err(e) => {
            tracing::debug!(error = %e, "[task_sources:bus] list_sources failed, skipping");
            return;
        }
    };

    for source in sources
        .into_iter()
        .filter(|s| s.enabled && s.provider == provider)
    {
        // If the source pins a specific connection, only fire for it.
        if let Some(pinned) = source.connection_id.as_deref() {
            if pinned != connection_id {
                continue;
            }
        }
        tracing::info!(
            source_id = %source.id,
            toolkit = %toolkit,
            "[task_sources:bus] connection created → one-shot fetch"
        );
        // Spawn each fetch independently so the event handler does not
        // block dispatch on N sequential network round-trips (same
        // pattern as the periodic poll). Each fetch captures its own
        // owned config + source.
        let config = config.clone();
        let ctx = ctx.clone();
        // Scoped: the fetch keeps the scope's agent context.
        ctx.clone().spawn(async move {
            let _ =
                pipeline::run_source_once(&ctx, &config, &source, FetchReason::ConnectionCreated)
                    .await;
        });
    }
}

/// Register the connection subscriber. Idempotent — the handle is held
/// in a process-global `OnceLock` so it is never dropped (which would
/// cancel the subscription).
pub fn register_task_sources_subscriber() {
    if CONNECTION_HANDLE.get().is_some() {
        return;
    }
    match BUS.subscribe(Arc::new(TaskSourcesConnectionSubscriber)) {
        Some(handle) => {
            let _ = CONNECTION_HANDLE.set(handle);
            tracing::debug!("[task_sources:bus] connection subscriber registered");
        }
        None => {
            tracing::warn!(
                "[task_sources:bus] event bus not initialized; subscriber not registered"
            );
        }
    }
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;
