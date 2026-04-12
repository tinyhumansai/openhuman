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
//! The triage path is gated on `OPENHUMAN_TRIGGER_TRIAGE_ENABLED` (set
//! to `1`/`true`/`yes` to enable). Until commit 3 flips the default on,
//! production builds fall through to logging-only behaviour so there
//! is no risk of unexpected LLM traffic from composio webhooks.
//!
//! Keeping this logic behind the bus means the socket transport stays
//! dumb, and adding new consumers (UI push, skill triggers, automation
//! engines) is a one-line subscribe call.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

use crate::core::event_bus::{DomainEvent, EventHandler, SubscriptionHandle};
use crate::openhuman::agent::triage::{apply_decision, run_triage, TriggerEnvelope};

/// Env var that **disables** the triage pipeline. The pipeline is
/// enabled by default; set to `1`/`true`/`yes` to opt out (e.g. for
/// debugging or in environments where LLM calls on every Composio
/// webhook are undesirable).
const TRIAGE_DISABLED_ENV: &str = "OPENHUMAN_TRIGGER_TRIAGE_DISABLED";

static COMPOSIO_TRIGGER_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Register the long-lived composio trigger subscriber on the global
/// event bus. Idempotent.
pub fn register_composio_trigger_subscriber() {
    if COMPOSIO_TRIGGER_HANDLE.get().is_some() {
        return;
    }
    match crate::core::event_bus::subscribe_global(Arc::new(ComposioTriggerSubscriber::new())) {
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
            "[composio] trigger received"
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
}
