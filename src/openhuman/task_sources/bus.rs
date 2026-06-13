//! Event-bus integration for the `task_sources` domain.
//!
//! Subscribes to `ComposioConnectionCreated`: when a user connects a
//! toolkit that has matching enabled task sources, fire a one-shot
//! `ConnectionCreated` fetch so freshly-connected work shows up without
//! waiting for the next periodic tick.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

use crate::core::event_bus::{subscribe_global, DomainEvent, EventHandler, SubscriptionHandle};
use crate::openhuman::config::rpc as config_rpc;

use super::types::{FetchReason, ProviderSlug};
use super::{pipeline, store};

static CONNECTION_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Fires a one-shot fetch for matching sources when a Composio
/// connection is created.
pub struct TaskSourcesConnectionSubscriber;

#[async_trait]
impl EventHandler for TaskSourcesConnectionSubscriber {
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
            ..
        } = event
        else {
            return;
        };

        // Only act for toolkits we model as task sources.
        let Ok(provider) = ProviderSlug::parse(toolkit) else {
            return;
        };

        let config = match config_rpc::load_config_with_timeout().await {
            Ok(config) => config,
            Err(e) => {
                tracing::debug!(error = %e, "[task_sources:bus] load_config failed, skipping");
                return;
            }
        };
        if !config.task_sources.enabled {
            return;
        }

        let sources = match store::list_sources(&config) {
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
            tokio::spawn(async move {
                let _ = pipeline::run_source_once(&config, &source, FetchReason::ConnectionCreated)
                    .await;
            });
        }
    }
}

/// Register the connection subscriber. Idempotent — the handle is held
/// in a process-global `OnceLock` so it is never dropped (which would
/// cancel the subscription).
pub fn register_task_sources_subscriber() {
    if CONNECTION_HANDLE.get().is_some() {
        return;
    }
    match subscribe_global(Arc::new(TaskSourcesConnectionSubscriber)) {
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
