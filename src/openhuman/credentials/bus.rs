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

        // (1) Stand down background workers immediately — before any async work.
        //     Cheap atomic flip; safe to call repeatedly from concurrent publishers.
        //     We may override this back to `false` below if the current session
        //     turns out to be a local offline session (never truly expired).
        scheduler_gate::set_signed_out(true);

        match crate::openhuman::config::rpc::load_config_with_timeout().await {
            Ok(config) => {
                let is_local_session = crate::api::jwt::get_session_token(&config)
                    .ok()
                    .flatten()
                    .is_some_and(|token| {
                        crate::openhuman::credentials::session_support::is_local_session_token(
                            &token,
                        )
                    });
                if is_local_session {
                    tracing::warn!(
                        source = %source,
                        reason = %reason,
                        "[auth] SessionExpired ignored for local offline session — re-enabling scheduler gate"
                    );
                    // Undo the eager flip: local sessions are never truly expired.
                    scheduler_gate::set_signed_out(false);
                    return;
                }

                tracing::warn!(
                    source = %source,
                    reason = %reason,
                    "[auth] SessionExpired received — pausing background LLM work and clearing session"
                );

                // (2) Tear down the session. We must call clear_session against a
                //     loaded config; if the config can't load (rare — disk issue),
                //     we've at least pinned the scheduler gate so background work
                //     can't make things worse.
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
                // set_signed_out(true) was already called above; scheduler gate
                // is pinned even though we cannot clear the JWT this cycle.
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
