//! Event bus handlers for the credentials / auth domain.
//!
//! The [`SessionExpiredSubscriber`] listens for [`DomainEvent::SessionExpired`]
//! events (published from any 401-detection site — `jsonrpc.invoke_method`,
//! `llm_provider.api_error`, …) and runs the canonical teardown:
//!
//! 1. Flip the scheduler-gate signed-out override so every existing
//!    background worker stalls at its next `wait_for_capacity()` call
//!    instead of firing more requests at a backend that will only ever
//!    401 them. We flip **before** `clear_session` so any work that
//!    re-enters the gate during teardown also stalls.
//! 2. Call [`clear_session`] to remove the stored JWT, clear the
//!    active-user marker, and stop login-gated services
//!    (voice / autocomplete / screen intelligence / local AI / dictation /
//!    subconscious). Idempotent — repeat events are safe.
//!
//! Without this subscriber, a 401 from a background LLM call would only
//! be detected but never acted on, and the same loop would 401 again on
//! the next iteration. This is the fix for issue
//! `OPENHUMAN-TAURI-1T` (5,414 Sentry events from one user's
//! cron-driven LLM calls after session expiry).

use crate::core::event_bus::{DomainEvent, EventHandler};
use crate::openhuman::scheduler_gate;
use async_trait::async_trait;

/// Subscribes to [`DomainEvent::SessionExpired`] and runs the canonical
/// session-teardown. Singleton — register once at startup.
pub struct SessionExpiredSubscriber;

impl Default for SessionExpiredSubscriber {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionExpiredSubscriber {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl EventHandler for SessionExpiredSubscriber {
    fn name(&self) -> &str {
        "credentials::session_expired_handler"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["auth"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::SessionExpired { source, reason } = event else {
            return;
        };

        tracing::warn!(
            source = %source,
            reason = %reason,
            "[auth] SessionExpired received — pausing background LLM work and clearing session"
        );

        // (1) Stand down background workers immediately. Cheap atomic — safe
        //     even if called repeatedly from concurrent publishers.
        scheduler_gate::set_signed_out(true);

        // (2) Tear down the session. We must call clear_session against a
        //     loaded config; if the config can't load (rare — disk issue),
        //     we've at least pinned the scheduler gate so background work
        //     can't make things worse.
        match crate::openhuman::config::rpc::load_config_with_timeout().await {
            Ok(config) => {
                if let Err(err) = crate::openhuman::credentials::rpc::clear_session(&config).await {
                    tracing::warn!(
                        source = %source,
                        error = %err,
                        "[auth] clear_session failed during SessionExpired handling"
                    );
                } else {
                    tracing::info!(
                        source = %source,
                        "[auth] session cleared in response to SessionExpired"
                    );
                }
            }
            Err(err) => {
                tracing::warn!(
                    source = %source,
                    error = %err,
                    "[auth] could not load config during SessionExpired handling — scheduler gate is signed-out, but session JWT was not cleared"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_stable() {
        let s = SessionExpiredSubscriber::new();
        assert_eq!(s.name(), "credentials::session_expired_handler");
    }

    #[test]
    fn domain_filter_is_auth() {
        let s = SessionExpiredSubscriber::new();
        assert_eq!(s.domains(), Some(&["auth"][..]));
    }

    #[tokio::test]
    async fn handle_ignores_non_auth_events() {
        // Domain filter is advisory — the broadcast bus still delivers all
        // events to every subscriber. The handler must guard internally.
        let s = SessionExpiredSubscriber::new();
        // Reset state we depend on.
        scheduler_gate::set_signed_out(false);
        s.handle(&DomainEvent::SystemStartup {
            component: "test".into(),
        })
        .await;
        assert!(
            !scheduler_gate::is_signed_out(),
            "non-auth event must not flip the override"
        );
    }
}
