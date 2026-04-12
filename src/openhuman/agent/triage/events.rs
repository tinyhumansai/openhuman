//! Tiny wrappers around `publish_global` that keep the field list for
//! the three `Trigger*` `DomainEvent` variants in one place.
//!
//! The point is so that `evaluator.rs` and `escalation.rs` never touch
//! `DomainEvent::TriggerEvaluated { … }` directly — they call these
//! helpers, and the field layout can evolve (or we can start including
//! defaults like `source: envelope.source.slug().into()`) without
//! fanning out a churning diff.

use crate::core::event_bus::{publish_global, DomainEvent};

use super::envelope::TriggerEnvelope;

/// Publish [`DomainEvent::TriggerEvaluated`] for the given envelope.
/// Fires for *every* triage run, regardless of action.
pub fn publish_evaluated(
    envelope: &TriggerEnvelope,
    decision: &str,
    used_local: bool,
    latency_ms: u64,
) {
    publish_global(DomainEvent::TriggerEvaluated {
        source: envelope.source.slug().to_string(),
        external_id: envelope.external_id.clone(),
        display_label: envelope.display_label.clone(),
        decision: decision.to_string(),
        used_local,
        latency_ms,
    });
}

/// Publish [`DomainEvent::TriggerEscalated`] — fired only on
/// `react`/`escalate`, *in addition* to `TriggerEvaluated`.
pub fn publish_escalated(envelope: &TriggerEnvelope, target_agent: &str) {
    publish_global(DomainEvent::TriggerEscalated {
        source: envelope.source.slug().to_string(),
        external_id: envelope.external_id.clone(),
        display_label: envelope.display_label.clone(),
        target_agent: target_agent.to_string(),
    });
}

/// Publish [`DomainEvent::TriggerEscalationFailed`] — fired when the
/// whole pipeline gave up (both local and remote failed, or the
/// classifier reply couldn't be parsed after a retry).
pub fn publish_failed(envelope: &TriggerEnvelope, reason: &str) {
    publish_global(DomainEvent::TriggerEscalationFailed {
        source: envelope.source.slug().to_string(),
        external_id: envelope.external_id.clone(),
        reason: reason.to_string(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::event_bus::{global, init_global, DomainEvent};
    use crate::openhuman::agent::triage::TriggerEnvelope;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn publish_helpers_emit_expected_trigger_events() {
        let _ = init_global(32);
        let seen = Arc::new(Mutex::new(Vec::<DomainEvent>::new()));
        let seen_handler = Arc::clone(&seen);
        let _handle = global().unwrap().on("triage-events-test", move |event| {
            let seen = Arc::clone(&seen_handler);
            let cloned = event.clone();
            Box::pin(async move {
                seen.lock().await.push(cloned);
            })
        });

        let envelope = TriggerEnvelope::from_composio(
            "gmail",
            "GMAIL_NEW_GMAIL_MESSAGE",
            "trig-events",
            "evt-123",
            json!({ "subject": "Coverage" }),
        );

        publish_evaluated(&envelope, "acknowledge", true, 42);
        publish_escalated(&envelope, "trigger_reactor");
        publish_failed(&envelope, "boom");

        sleep(Duration::from_millis(20)).await;

        let captured = seen.lock().await;
        assert!(captured.iter().any(|event| matches!(
            event,
            DomainEvent::TriggerEvaluated {
                source,
                external_id,
                decision,
                used_local,
                latency_ms,
                ..
            } if source == "composio"
                && external_id == "evt-123"
                && decision == "acknowledge"
                && *used_local
                && *latency_ms == 42
        )));
        assert!(captured.iter().any(|event| matches!(
            event,
            DomainEvent::TriggerEscalated {
                external_id,
                target_agent,
                ..
            } if external_id == "evt-123" && target_agent == "trigger_reactor"
        )));
        assert!(captured.iter().any(|event| matches!(
            event,
            DomainEvent::TriggerEscalationFailed {
                external_id,
                reason,
                ..
            } if external_id == "evt-123" && reason == "boom"
        )));
    }
}
