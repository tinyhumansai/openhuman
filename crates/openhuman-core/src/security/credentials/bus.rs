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
//!    (voice / autocomplete / local AI / dictation).
//!    Idempotent — repeat events are safe.
//!
//! In SaaS mode the handler does nothing process-wide: each user's
//! credential is theirs, and the gateway refreshes it.
//!
//! Without this subscriber, a 401 from a background LLM call would only
//! be detected but never acted on, and the same loop would 401 again on
//! the next iteration. This is the fix for issue
//! `OPENHUMAN-TAURI-1T` (5,414 Sentry events from one user's
//! cron-driven LLM calls after session expiry).

use crate::core::events::DomainEvent;
use crate::core::runtime::DispatchCtx;
use crate::platform::llm_gate;
use async_trait::async_trait;
use tinybus::EventHandler;

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
impl EventHandler<DomainEvent> for SessionExpiredSubscriber {
    fn name(&self) -> &str {
        "credentials::session_expired_handler"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["auth"])
    }

    async fn handle(&self, event: &DomainEvent) {
        // A session expiry belongs to the host's own sign-in: outside SaaS the
        // process default, and in SaaS (which leaves it to the gateway) none.
        self.handle_with(
            &DispatchCtx::host_default(),
            crate::core::runtime::is_saas(),
            event,
        )
        .await;
    }
}

impl SessionExpiredSubscriber {
    /// [`EventHandler::handle`] with the process mode passed in, so the SaaS
    /// early return is testable without the process-wide mode lock.
    pub(crate) async fn handle_with(&self, ctx: &DispatchCtx, saas: bool, event: &DomainEvent) {
        let DomainEvent::SessionExpired { source, reason } = event else {
            return;
        };

        // SaaS: the credential belongs to one user and the gateway owns its
        // refresh (`profiles.set_credential`). Nothing process-wide is torn
        // down; the failing call already reports the 401 to that user.
        if saas {
            tracing::warn!(
                source = %source,
                reason = %reason,
                "[auth] SessionExpired in SaaS mode — left to the gateway; no process-wide teardown"
            );
            return;
        }

        // (1) Stand down background workers immediately — before any async work.
        //     Cheap atomic flip; safe to call repeatedly from concurrent publishers.
        //     We may override this back to `false` below if the current session
        //     turns out to be a local offline session (never truly expired).
        llm_gate::set_signed_out(true);

        match ctx.load_config().await {
            Ok(config) => {
                // An API-key runtime has no session to clear: a backend 401 on
                // the key is a per-call error for the caller, not a sign-out.
                if crate::security::credentials::api_key::has_api_key(&config) {
                    tracing::warn!(
                        source = %source,
                        reason = %reason,
                        "[auth] SessionExpired ignored — runtime authenticates with an API key; re-enabling scheduler gate"
                    );
                    llm_gate::set_signed_out(false);
                    return;
                }
                let is_local_session =
                    crate::security::credentials::jwt::get_session_token(&config)
                        .ok()
                        .flatten()
                        .is_some_and(|token| {
                            crate::security::credentials::session_support::is_local_session_token(
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
                    llm_gate::set_signed_out(false);
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
                if let Err(err) =
                    crate::security::credentials::rpc::clear_session(ctx, &config).await
                {
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
#[path = "bus_tests.rs"]
mod tests;
