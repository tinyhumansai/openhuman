//! Drops and eagerly re-warms the integrations cache when the user flips
//! Composio between backend and direct mode.

use async_trait::async_trait;
use tinybus::EventHandler;

use crate::config::rpc as config_rpc;
use crate::core::events::DomainEvent;
use crate::integrations::composio::ops;
use crate::integrations::composio::FetchConnectedIntegrationsStatus;

/// Drops the prompt-level integrations cache whenever the user flips
/// `config.composio().mode` between `"backend"` and `"direct"` or
/// stores/clears the direct-mode API key. Without this, the chat
/// runtime keeps the old tenant's tool catalogue / connection list
/// pinned until another event or process restart — that's the regression behind
/// "I switched to Direct and my old integrations are still showing"
/// (#1710).
///
/// The subscriber is intentionally tiny: it only clears the cache,
/// then attempts a best-effort eager warm + `ComposioIntegrationsChanged`
/// publish in a detached task so active sessions can refresh their
/// delegation schema without waiting for the next turn boundary.
///
/// The warm/publish step is intentionally opportunistic: if config load
/// or backend access fails we leave the cache cold and rely on the
/// existing 5 s UI poll / next-turn fallback path.
pub struct ComposioConfigChangedSubscriber;

impl ComposioConfigChangedSubscriber {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ComposioConfigChangedSubscriber {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventHandler<DomainEvent> for ComposioConfigChangedSubscriber {
    fn name(&self) -> &str {
        "composio::config_changed"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["composio"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::ComposioConfigChanged { mode, api_key_set } = event else {
            return;
        };

        tracing::info!(
            mode = %mode,
            api_key_set = api_key_set,
            "[composio-cache] config changed — invalidating integrations cache"
        );
        ops::invalidate_connected_integrations_cache();

        tokio::spawn(async move {
            let config = match config_rpc::load_config_with_timeout().await {
                Ok(config) => config,
                Err(error) => {
                    tracing::debug!(
                        error = %error,
                        "[composio-cache] config changed eager warm skipped: config load failed"
                    );
                    return;
                }
            };

            match ops::fetch_connected_integrations_status(&config).await {
                FetchConnectedIntegrationsStatus::Authoritative(entries) => {
                    super::publish_integrations_changed(&entries);
                    tracing::debug!(
                        "[composio-cache] config changed eager warm complete; published integrations changed"
                    );
                }
                FetchConnectedIntegrationsStatus::Unavailable => {
                    tracing::debug!(
                        "[composio-cache] config changed eager warm skipped: backend unavailable"
                    );
                }
            }
        });
    }
}
