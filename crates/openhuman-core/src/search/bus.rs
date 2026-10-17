//! Event subscriber that keeps the TinySearch module's managed routes in step
//! with the credential this core holds.
//!
//! The module receives the backend credential privately at (re)initialization.
//! Without this subscriber a login would only reach it on the next search
//! call, and a logout would leave the old credential in the module until then.

use crate::core::runtime::DispatchCtx;
use async_trait::async_trait;
use tinybus::EventHandler;

use crate::core::events::DomainEvent;

/// Refreshes a loaded TinySearch module on [`DomainEvent::CredentialChanged`].
#[derive(Debug, Default)]
pub struct CredentialRefreshSubscriber;

#[async_trait]
impl EventHandler<DomainEvent> for CredentialRefreshSubscriber {
    fn name(&self) -> &str {
        "search::credential_refresh"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["auth"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::CredentialChanged { kind, tenant } = event else {
            return;
        };
        let Some(ctx) = DispatchCtx::for_event(tenant.as_ref()) else {
            tracing::debug!(kind = %kind, "[search][bus] credential change has no tenant; skipping refresh");
            return;
        };
        let config = match ctx.load_config().await {
            Ok(config) => config,
            Err(error) => {
                tracing::warn!(kind = %kind, %error, "[search][bus] config unavailable; skipping refresh");
                return;
            }
        };
        match crate::modules::search::refresh_loaded(&ctx, &config).await {
            Ok(()) => {
                tracing::debug!(kind = %kind, "[search][bus] module refreshed after credential change")
            }
            Err(error) => {
                tracing::warn!(kind = %kind, %error, "[search][bus] module refresh failed")
            }
        }
    }
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;
