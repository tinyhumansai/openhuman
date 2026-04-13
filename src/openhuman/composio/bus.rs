//! Event bus subscribers for the Composio domain.
//!
//! The backend emits `composio:trigger` over Socket.IO when a webhook
//! arrives and is HMAC-verified (see
//! `src/controllers/agentIntegrations/composio/handleWebhook.ts` in the
//! backend repo). The socket transport layer parses that payload and
//! publishes [`DomainEvent::ComposioTriggerReceived`], and this
//! subscriber is what actually does something with it.
//!
//! ## What it does today
//!
//! - **Always**: logs the trigger at `debug` level for grep-friendly
//!   audit trails.
//! - **When enabled**: runs the trigger through
//!   [`crate::openhuman::agent::triage::run_triage`] to produce a
//!   [`TriageDecision`] and then
//!   [`crate::openhuman::agent::triage::apply_decision`] to act on it.
//!   The classifier runs on the shared built-in
//!   [`trigger_triage`][trigger_triage] agent and its decisions are
//!   published as `TriggerEvaluated` / `TriggerEscalated` events on
//!   the bus.
//!
//! [trigger_triage]: crate::openhuman::agent::agents
//!
//! ## Feature flag
//!
//! The triage path is gated on `OPENHUMAN_TRIGGER_TRIAGE_DISABLED` (set
//! to `1`/`true`/`yes` to disable). The pipeline is on by default; the
//! env var is an opt-out escape hatch.
//!
//! There are two long-lived subscribers, both registered at startup:
//!
//!   * [`ComposioTriggerSubscriber`] — handles
//!     [`DomainEvent::ComposioTriggerReceived`]. The backend HMAC-verifies
//!     a Composio webhook, parses it, and emits `composio:trigger` over
//!     Socket.IO; the socket transport publishes that as a domain event.
//!     The subscriber routes it through the triage pipeline.
//!
//!   * [`ComposioConnectionCreatedSubscriber`] — handles
//!     [`DomainEvent::ComposioConnectionCreated`]. Fired by `composio_authorize`
//!     once the OAuth handoff has produced a `connectUrl` + `connectionId`.
//!     We look up the provider and call `on_connection_created`, which
//!     by default fetches the user profile and runs the initial sync.
//!
//! Both subscribers do their work in a `tokio::spawn`-ed task so the
//! event bus dispatch loop is never blocked by a long-running provider
//! call (sync can take seconds).

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;

use crate::core::event_bus::{subscribe_global, DomainEvent, EventHandler, SubscriptionHandle};
use crate::openhuman::agent::triage::{apply_decision, run_triage, TriggerEnvelope};
use crate::openhuman::config::rpc as config_rpc;

use super::client::ComposioClient;
use super::providers::{get_provider, ProviderContext};

/// Env var that **disables** the triage pipeline. The pipeline is
/// enabled by default; set to `1`/`true`/`yes` to opt out (e.g. for
/// debugging or in environments where LLM calls on every Composio
/// webhook are undesirable).
const TRIAGE_DISABLED_ENV: &str = "OPENHUMAN_TRIGGER_TRIAGE_DISABLED";

/// How long we'll keep polling the backend after `composio_authorize`
/// returns a `connectUrl`, waiting for the user to actually finish the
/// hosted OAuth flow and the connection to flip to ACTIVE/CONNECTED.
/// One minute matches typical hosted-OAuth round-trip times and is
/// generous enough to absorb a slow tab-switch + login + consent.
const CONNECTION_READY_TIMEOUT: Duration = Duration::from_secs(60);

/// Poll backoff schedule (start, max). We start aggressive so the
/// fast-path (user already had the tab open) feels immediate, then
/// back off so we don't hammer the backend during the long tail of
/// users who actually have to log in to the upstream service.
const CONNECTION_READY_INITIAL_BACKOFF: Duration = Duration::from_millis(500);
const CONNECTION_READY_MAX_BACKOFF: Duration = Duration::from_secs(4);

static COMPOSIO_TRIGGER_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();
static COMPOSIO_CONNECTION_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Register both long-lived composio subscribers on the global event
/// bus, and initialise the default provider registry. Idempotent.
pub fn register_composio_trigger_subscriber() {
    // Make sure the registry is populated before any event arrives —
    // otherwise the very first webhook would no-op because the
    // subscriber's `get_provider` lookup would miss.
    super::providers::init_default_providers();

    if COMPOSIO_TRIGGER_HANDLE.get().is_none() {
        match subscribe_global(Arc::new(ComposioTriggerSubscriber::new())) {
            Some(handle) => {
                let _ = COMPOSIO_TRIGGER_HANDLE.set(handle);
                log::debug!("[event_bus] composio trigger subscriber registered");
            }
            None => {
                log::warn!(
                    "[event_bus] failed to register composio trigger subscriber — bus not initialized"
                );
            }
        }
    }

    if COMPOSIO_CONNECTION_HANDLE.get().is_none() {
        match subscribe_global(Arc::new(ComposioConnectionCreatedSubscriber::new())) {
            Some(handle) => {
                let _ = COMPOSIO_CONNECTION_HANDLE.set(handle);
                log::debug!("[event_bus] composio connection_created subscriber registered");
            }
            None => {
                log::warn!(
                    "[event_bus] failed to register composio connection_created subscriber — bus not initialized"
                );
            }
        }
    }
}

/// Logs and (when enabled) routes `ComposioTriggerReceived` events
/// through the reusable `agent::triage` pipeline.
pub struct ComposioTriggerSubscriber;

impl ComposioTriggerSubscriber {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ComposioTriggerSubscriber {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventHandler for ComposioTriggerSubscriber {
    fn name(&self) -> &str {
        "composio::trigger"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["composio"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::ComposioTriggerReceived {
            toolkit,
            trigger,
            metadata_id,
            metadata_uuid,
            payload,
        } = event
        else {
            return;
        };

        tracing::debug!(
            toolkit = %toolkit,
            trigger = %trigger,
            id = %metadata_id,
            uuid = %metadata_uuid,
            payload_bytes = payload.to_string().len(),
            "[composio:bus] trigger received"
        );

        if triage_disabled() {
            tracing::debug!(
                toolkit = %toolkit,
                trigger = %trigger,
                "[composio][triage] skipped: {TRIAGE_DISABLED_ENV} is set"
            );
            return;
        }

        // Build the envelope outside the spawned task so any panic in
        // `from_composio` surfaces on the bus dispatch thread (where
        // the broadcast subscriber loop can log it) rather than being
        // swallowed inside a detached task.
        let envelope = TriggerEnvelope::from_composio(
            toolkit,
            trigger,
            metadata_id,
            metadata_uuid,
            payload.clone(),
        );
        tracing::debug!(
            label = %envelope.display_label,
            external_id = %envelope.external_id,
            "[composio][triage] dispatching to agent::triage::run_triage"
        );

        // Spawn so the bus dispatch loop stays non-blocking — the
        // triage turn is an LLM round-trip that may take seconds.
        tokio::spawn(async move {
            match run_triage(&envelope).await {
                Ok(run) => {
                    if let Err(e) = apply_decision(run, &envelope).await {
                        tracing::error!(
                            label = %envelope.display_label,
                            error = %e,
                            "[composio][triage] apply_decision failed"
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(
                        label = %envelope.display_label,
                        error = %e,
                        "[composio][triage] run_triage failed"
                    );
                }
            }
        });
    }
}

/// Returns `true` when `OPENHUMAN_TRIGGER_TRIAGE_DISABLED` is set to a
/// truthy value. The pipeline is **on by default**; this env var is the
/// opt-out escape hatch.
fn triage_disabled() -> bool {
    matches!(
        std::env::var(TRIAGE_DISABLED_ENV).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

// ── Connection-created subscriber ───────────────────────────────────

/// Routes `ComposioConnectionCreated` events to the toolkit's provider.
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
impl EventHandler for ComposioConnectionCreatedSubscriber {
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

        // Bust the prompt-level integrations cache so the next agent
        // session picks up the newly connected toolkit.
        super::ops::invalidate_connected_integrations_cache();

        tracing::info!(
            toolkit = %toolkit,
            connection_id = %connection_id,
            "[composio:bus] connection_created"
        );

        let Some(provider) = get_provider(toolkit) else {
            tracing::debug!(
                toolkit = %toolkit,
                "[composio:bus] no provider registered, skipping connection_created hook"
            );
            return;
        };

        let toolkit = toolkit.clone();
        let connection_id = connection_id.clone();

        tokio::spawn(async move {
            // The OAuth handoff is asynchronous — the backend returned
            // a `connectUrl` and we published the event before the user
            // has actually clicked through. Resolve the config + client
            // first, then poll the backend for the connection record
            // until we observe ACTIVE/CONNECTED (or hit the timeout).
            // Only then do we run the provider hook, so the very first
            // provider call doesn't race the OAuth handshake.
            //
            // NOTE: Future improvement — listen for an explicit
            // "connection_active" backend event instead of polling.
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
            let Some(ctx) = ProviderContext::from_config(
                Arc::new(config),
                toolkit.clone(),
                Some(connection_id.clone()),
            ) else {
                tracing::debug!(
                    toolkit = %toolkit,
                    "[composio:bus] no composio client (not signed in?), skipping hook"
                );
                return;
            };

            match wait_for_connection_active(&ctx.client, &connection_id).await {
                Ok(status) => {
                    tracing::info!(
                        toolkit = %toolkit,
                        connection_id = %connection_id,
                        status = %status,
                        "[composio:bus] connection observed active, dispatching on_connection_created"
                    );
                }
                Err(WaitError::Timeout { last_status }) => {
                    tracing::warn!(
                        toolkit = %toolkit,
                        connection_id = %connection_id,
                        last_status = ?last_status,
                        timeout_secs = CONNECTION_READY_TIMEOUT.as_secs(),
                        "[composio:bus] timed out waiting for connection to become active; aborting on_connection_created"
                    );
                    return;
                }
                Err(WaitError::Lookup { error }) => {
                    tracing::warn!(
                        toolkit = %toolkit,
                        connection_id = %connection_id,
                        error = %error,
                        "[composio:bus] backend lookup failed while waiting for connection; aborting on_connection_created"
                    );
                    return;
                }
            }

            if let Err(e) = provider.on_connection_created(&ctx).await {
                tracing::warn!(
                    toolkit = %toolkit,
                    connection_id = %connection_id,
                    error = %e,
                    "[composio:bus] provider on_connection_created failed"
                );
            } else {
                // Successful connection-created sync — record the
                // timestamp so the periodic scheduler doesn't
                // immediately re-fire for this connection.
                super::periodic::record_sync_success(&toolkit, &connection_id);
            }
        });
    }
}

// ── Connection-readiness polling ────────────────────────────────────

#[derive(Debug)]
enum WaitError {
    /// Polling exhausted [`CONNECTION_READY_TIMEOUT`] without observing
    /// the connection in an active state. `last_status` is whatever the
    /// backend last reported (e.g. `"INITIATED"`, `"PENDING"`).
    Timeout { last_status: Option<String> },
    /// The backend lookup itself errored — we treat that as fatal for
    /// this dispatch (no point spinning when `list_connections` is
    /// unreachable).
    Lookup { error: String },
}

/// Poll the backend for `connection_id` until it appears with an
/// `ACTIVE` or `CONNECTED` status, or until we hit
/// [`CONNECTION_READY_TIMEOUT`]. Backoff is exponential between
/// [`CONNECTION_READY_INITIAL_BACKOFF`] and
/// [`CONNECTION_READY_MAX_BACKOFF`].
///
/// On success returns the observed status string. On timeout returns
/// the last status we saw (helpful for "stuck in INITIATED" debugging).
async fn wait_for_connection_active(
    client: &ComposioClient,
    connection_id: &str,
) -> Result<String, WaitError> {
    let started = std::time::Instant::now();
    let mut backoff = CONNECTION_READY_INITIAL_BACKOFF;
    let mut last_status: Option<String> = None;

    loop {
        match client.list_connections().await {
            Ok(resp) => {
                if let Some(conn) = resp.connections.into_iter().find(|c| c.id == connection_id) {
                    if matches!(conn.status.as_str(), "ACTIVE" | "CONNECTED") {
                        return Ok(conn.status);
                    }
                    last_status = Some(conn.status);
                }
                // Connection not found yet — backend may not have
                // persisted it to its index. Treat the same as a
                // not-yet-active status and retry.
            }
            Err(e) => {
                // One transient lookup failure shouldn't kill the
                // dispatch — keep polling until the timeout.
                tracing::debug!(
                    connection_id = %connection_id,
                    error = %e,
                    "[composio:bus] list_connections failed during readiness poll (will retry)"
                );
                last_status = last_status.or_else(|| Some(format!("lookup_error: {e}")));
            }
        }

        if started.elapsed() >= CONNECTION_READY_TIMEOUT {
            // If we never even got a successful lookup, propagate that
            // as a Lookup error rather than Timeout so the caller can
            // distinguish "user is taking forever" from "backend is
            // down".
            if let Some(ref status) = last_status {
                if status.starts_with("lookup_error:") {
                    return Err(WaitError::Lookup {
                        error: status.clone(),
                    });
                }
            }
            return Err(WaitError::Timeout { last_status });
        }

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(CONNECTION_READY_MAX_BACKOFF);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn ignores_non_composio_events() {
        let sub = ComposioTriggerSubscriber::new();
        sub.handle(&DomainEvent::CronJobTriggered {
            job_id: "j1".into(),
            job_type: "shell".into(),
        })
        .await;
        // No panic = pass.
    }

    #[tokio::test]
    async fn handles_trigger_event_without_panic() {
        // Disable triage so this test takes the log-only path and
        // doesn't spawn a real LLM turn.
        std::env::set_var(TRIAGE_DISABLED_ENV, "1");
        let sub = ComposioTriggerSubscriber::new();
        sub.handle(&DomainEvent::ComposioTriggerReceived {
            toolkit: "gmail".into(),
            trigger: "GMAIL_NEW_GMAIL_MESSAGE".into(),
            metadata_id: "trig-1".into(),
            metadata_uuid: "uuid-1".into(),
            payload: json!({ "from": "a@b.com", "subject": "hi" }),
        })
        .await;
        std::env::remove_var(TRIAGE_DISABLED_ENV);
    }

    #[test]
    fn triage_disabled_flag_parser() {
        // Truthy values disable triage.
        for val in ["1", "true", "TRUE", "yes", "YES"] {
            std::env::set_var(TRIAGE_DISABLED_ENV, val);
            assert!(triage_disabled(), "expected '{val}' to disable triage");
        }
        // Non-truthy values leave triage on.
        for val in ["", "0", "false", "off"] {
            std::env::set_var(TRIAGE_DISABLED_ENV, val);
            assert!(!triage_disabled(), "expected '{val}' to keep triage on");
        }
        // Unset = triage on (default).
        std::env::remove_var(TRIAGE_DISABLED_ENV);
        assert!(!triage_disabled(), "unset must default to triage enabled");
    }

    #[tokio::test]
    async fn handles_connection_created_event_without_panic() {
        let sub = ComposioConnectionCreatedSubscriber::new();
        sub.handle(&DomainEvent::ComposioConnectionCreated {
            toolkit: "gmail".into(),
            connection_id: "conn-1".into(),
            connect_url: "https://composio.example/connect/abc".into(),
        })
        .await;
    }
}
