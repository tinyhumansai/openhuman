//! Host extensions to [`tinymemory_core`]'s Composio provider context.
//!
//! The core's `ProviderContext` reaches Composio through the behavioural
//! `ComposioHost` seam and never names a client type. One caller needs more
//! than that: the connection-created subscriber in [`super::bus`] hands a
//! concrete `ComposioClient` to legacy helpers written against the old
//! `&ComposioClient` API (`slack::users::SlackUsers::fetch`,
//! `slack::provider::execute_with_retry`).
//!
//! Rather than widen the seam for one host-only caller, the method lives here
//! as an extension trait. `ProviderContext`'s fields are public, so it needs
//! nothing the core does not already expose.

use anyhow::anyhow;
use async_trait::async_trait;

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::integrations::composio::client::{
    create_composio_client, ComposioClient, ComposioClientKind,
};
use tinymemory_core::sync::composio::providers::ProviderContext;

/// Backend-tenant client access for provider helpers that have not been ported
/// to the mode-aware factory.
#[async_trait]
pub trait ProviderContextExt {
    /// A backend-tenant [`ComposioClient`].
    ///
    /// # Errors
    ///
    /// Returns `Err` when the live config selects direct mode — these legacy
    /// helpers were written against the backend-tenant client and have not been
    /// ported. Direct-mode users hit this as a hard error rather than silently
    /// routing through the wrong tenant.
    async fn backend_client(&self) -> anyhow::Result<ComposioClient>;
}

#[async_trait]
impl ProviderContextExt for ProviderContext {
    async fn backend_client(&self) -> anyhow::Result<ComposioClient> {
        // [#1710 Wave 4] Reload config fresh per call so a mid-session
        // `composio.mode` toggle takes effect immediately. The Arc<Config>
        // snapshot held by `self` was taken at agent-init time and is otherwise
        // stale relative to subsequent set_api_key / clear_api_key RPCs.
        //
        // Anchored to the snapshot's config_path (not OPENHUMAN_WORKSPACE) for
        // the same isolation reason as the core's `execute`.
        let live_config = config_rpc::reload_config_from_paths(
            self.config.config_path(),
            self.config.workspace_dir(),
        )
        .await
        .map_err(|e| {
            tracing::warn!(
                toolkit = %self.toolkit,
                error = %e,
                "[composio:provider_context] backend_client: reload_config failed"
            );
            anyhow!("composio provider_context.backend_client: failed to reload live config: {e}")
        })?;
        match create_composio_client(&live_config)? {
            ComposioClientKind::Backend(client) => Ok(client),
            ComposioClientKind::Direct(_) => Err(anyhow!(
                "composio direct mode is not yet supported on this provider's helper path; \
                 toolkit={}",
                self.toolkit
            )),
        }
    }
}
