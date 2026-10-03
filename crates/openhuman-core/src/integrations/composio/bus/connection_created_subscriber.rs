//! Routes `ComposioConnectionCreated` events: once the connection is
//! confirmed active, refreshes the integrations cache, then (after
//! onboarding) fetches the account profile and starts an initial
//! `composio_sync` for toolkits with a native sync provider.

use std::time::Duration;

use async_trait::async_trait;
use tinybus::EventHandler;

use crate::config::rpc as config_rpc;
use crate::config::Config;
use crate::core::events::DomainEvent;
use crate::integrations::composio::client::{resolve_composio_route, ComposioRoute};
use crate::integrations::composio::module_client::{self as connectors, methods};
use crate::integrations::composio::ops;
use crate::integrations::composio::types::ComposioConnectionsResponse;
use crate::integrations::composio::FetchConnectedIntegrationsStatus;

/// How long to keep polling after `composio_authorize` returns a
/// `connectUrl`, waiting for the user to finish the hosted OAuth flow.
const CONNECTION_READY_TIMEOUT: Duration = Duration::from_secs(60);

/// Poll backoff schedule (start, max): aggressive first so the fast path
/// feels immediate, then backing off for users who still have to log in.
const CONNECTION_READY_INITIAL_BACKOFF: Duration = Duration::from_millis(500);
const CONNECTION_READY_MAX_BACKOFF: Duration = Duration::from_secs(4);

/// Reloads the live config and requires it to route through the backend
/// tenant — the readiness probe is a backend-only metadata call, and routing
/// it through the wrong tenant is worse than refusing (#1710). Reloaded rather
/// than taken from the snapshot: the OAuth completion being reacted to may
/// have written credentials since.
async fn backend_composio_config(config: &Config, toolkit: &str) -> anyhow::Result<Config> {
    let live_config =
        config_rpc::reload_config_from_paths(&config.config_path, &config.workspace_dir)
            .await
            .map_err(|e| {
                anyhow::anyhow!("composio backend client: failed to reload live config: {e}")
            })?;
    match resolve_composio_route(&live_config)? {
        ComposioRoute::Backend => Ok(live_config),
        ComposioRoute::Direct(_) => Err(anyhow::anyhow!(
            "composio direct mode is not supported on this helper path; toolkit={toolkit}"
        )),
    }
}

/// Handles `ComposioConnectionCreated`.
pub struct ComposioConnectionCreatedSubscriber;

impl ComposioConnectionCreatedSubscriber {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ComposioConnectionCreatedSubscriber {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventHandler<DomainEvent> for ComposioConnectionCreatedSubscriber {
    fn name(&self) -> &str {
        "composio::connection_created"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["composio"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::ComposioConnectionCreated {
            toolkit,
            connection_id,
            connect_url: _,
        } = event
        else {
            return;
        };
        tracing::info!(
            toolkit = %toolkit,
            connection_id = %connection_id,
            "[composio:bus] connection_created"
        );
        // The cache refresh runs for every toolkit; only the profile fetch and
        // initial sync are gated on a native provider.
        let toolkit = toolkit.clone();
        let connection_id = connection_id.clone();
        tokio::spawn(async move { on_connection_created(toolkit, connection_id).await });
    }
}

async fn on_connection_created(toolkit: String, connection_id: String) {
    let config = match config_rpc::load_config_with_timeout().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                toolkit = %toolkit,
                error = %e,
                "[composio:bus] failed to load config for connection_created dispatch"
            );
            return;
        }
    };
    if resolve_composio_route(&config).is_err() {
        tracing::debug!(
            toolkit = %toolkit,
            "[composio:bus] no composio client (not signed in?), skipping hook"
        );
        return;
    }
    let live_config = match backend_composio_config(&config, &toolkit).await {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(
                toolkit = %toolkit,
                error = %e,
                "[composio:bus] backend client unavailable for connection-readiness poll; skipping"
            );
            return;
        }
    };
    match wait_for_connection_active(&live_config, &connection_id).await {
        Ok(status) => {
            tracing::info!(
                toolkit = %toolkit,
                connection_id = %connection_id,
                status = %status,
                "[composio:bus] connection observed active; invalidating + eagerly warming integrations cache"
            );
            ops::invalidate_connected_integrations_cache();
            match ops::fetch_connected_integrations_status(&live_config).await {
                FetchConnectedIntegrationsStatus::Authoritative(entries) => {
                    super::publish_integrations_changed(&entries);
                }
                FetchConnectedIntegrationsStatus::Unavailable => tracing::warn!(
                    toolkit = %toolkit,
                    connection_id = %connection_id,
                    "[composio:bus] eager cache warm after connection became active skipped: backend unavailable"
                ),
            }
        }
        Err(WaitError::Timeout { last_status }) => {
            tracing::warn!(
                toolkit = %toolkit,
                connection_id = %connection_id,
                last_status = ?last_status,
                timeout_secs = CONNECTION_READY_TIMEOUT.as_secs(),
                "[composio:bus] timed out waiting for connection to become active; skipping cache refresh + initial sync"
            );
            return;
        }
        Err(WaitError::Lookup { error }) => {
            tracing::warn!(
                toolkit = %toolkit,
                connection_id = %connection_id,
                error = %error,
                "[composio:bus] backend lookup failed while waiting for connection; skipping cache refresh + initial sync"
            );
            return;
        }
    }

    // Connections made during the setup wizard would otherwise spend cloud
    // credits on embedding before the user chose their AI routing (#3097).
    if !live_config.onboarding_completed {
        tracing::info!(
            toolkit = %toolkit,
            connection_id = %connection_id,
            "[composio:bus] onboarding not yet complete — skipping profile fetch and initial sync"
        );
        return;
    }
    if !crate::integrations::composio::providers::has_native_provider(&toolkit) {
        tracing::debug!(
            toolkit = %toolkit,
            "[composio:bus] no native sync provider for toolkit; skipping profile fetch and initial sync"
        );
        return;
    }
    if let Err(e) = ops::composio_get_user_profile(&live_config, &connection_id).await {
        tracing::warn!(
            toolkit = %toolkit,
            connection_id = %connection_id,
            error = %e,
            "[composio:bus] connection bootstrap (profile fetch) failed"
        );
    }
    match ops::composio_sync(
        &live_config,
        &connection_id,
        Some("connection_created".to_string()),
    )
    .await
    {
        Ok(_) => tracing::info!(
            toolkit = %toolkit,
            connection_id = %connection_id,
            "[composio:bus] initial sync started"
        ),
        Err(error) => tracing::debug!(
            toolkit = %toolkit,
            connection_id = %connection_id,
            error = %error,
            "[composio:bus] initial sync not started (memory off or unavailable)"
        ),
    }
}

// ── Connection-readiness polling ────────────────────────────────────

#[derive(Debug)]
pub(super) enum WaitError {
    /// Polling exhausted [`CONNECTION_READY_TIMEOUT`] without observing the
    /// connection active; `last_status` is what the backend last reported.
    Timeout { last_status: Option<String> },
    /// The backend lookup itself never succeeded.
    Lookup { error: String },
}

/// Polls `ListConnections` until `connection_id` is active, or until
/// [`CONNECTION_READY_TIMEOUT`]. Returns the observed status.
async fn wait_for_connection_active(
    config: &Config,
    connection_id: &str,
) -> Result<String, WaitError> {
    let started = std::time::Instant::now();
    let mut backoff = CONNECTION_READY_INITIAL_BACKOFF;
    let mut last_status: Option<String> = None;
    loop {
        match connectors::call_bare::<ComposioConnectionsResponse>(
            config,
            methods::LIST_CONNECTIONS,
        )
        .await
        {
            Ok(resp) => {
                if let Some(conn) = resp.connections.into_iter().find(|c| c.id == connection_id) {
                    if conn.is_active() {
                        return Ok(conn.status);
                    }
                    last_status = Some(conn.status);
                }
            }
            Err(e) => {
                tracing::debug!(
                    connection_id = %connection_id,
                    error = %e,
                    "[composio:bus] list_connections failed during readiness poll (will retry)"
                );
                last_status = last_status.or_else(|| Some(format!("lookup_error: {e}")));
            }
        }
        if started.elapsed() >= CONNECTION_READY_TIMEOUT {
            if let Some(status) = last_status
                .as_ref()
                .filter(|s| s.starts_with("lookup_error:"))
            {
                return Err(WaitError::Lookup {
                    error: status.clone(),
                });
            }
            return Err(WaitError::Timeout { last_status });
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(CONNECTION_READY_MAX_BACKOFF);
    }
}
