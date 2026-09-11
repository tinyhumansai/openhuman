//! Tiny wrappers around `publish_global` that keep the field list for
//! the three `Trigger*` `DomainEvent` variants in one place.
//!
//! The point is so that `evaluator.rs` and `escalation.rs` never touch
//! `DomainEvent::TriggerEvaluated { … }` directly — they call these
//! helpers, and the field layout can evolve (or we can start including
//! defaults like `source: envelope.source.slug().into()`) without
//! fanning out a churning diff.

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;

use super::envelope::TriggerEnvelope;

#[cfg(test)]
static TEST_EVENTS: std::sync::Mutex<Vec<DomainEvent>> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
fn record_test_event(event: &DomainEvent) {
    TEST_EVENTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(event.clone());
}

#[cfg(test)]
pub(crate) fn test_events_for_external_id(external_id: &str) -> Vec<DomainEvent> {
    TEST_EVENTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .filter(|event| match event {
            DomainEvent::TriggerEvaluated {
                external_id: event_external_id,
                ..
            }
            | DomainEvent::TriggerEscalated {
                external_id: event_external_id,
                ..
            }
            | DomainEvent::TriggerEscalationFailed {
                external_id: event_external_id,
                ..
            } => event_external_id == external_id,
            _ => false,
        })
        .cloned()
        .collect()
}

#[cfg(test)]
pub(crate) fn clear_test_events() {
    TEST_EVENTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
}

#[cfg(not(test))]
fn record_test_event(_event: &DomainEvent) {}

/// Publish [`DomainEvent::TriggerEvaluated`] for the given envelope.
/// Fires for *every* triage run, regardless of action.
pub fn publish_evaluated(
    envelope: &TriggerEnvelope,
    decision: &str,
    used_local: bool,
    latency_ms: u64,
) {
    let event = DomainEvent::TriggerEvaluated {
        source: envelope.source.slug().to_string(),
        external_id: envelope.external_id.clone(),
        display_label: envelope.display_label.clone(),
        decision: decision.to_string(),
        used_local,
        latency_ms,
    };
    record_test_event(&event);
    BUS.publish(event);
}

/// Publish [`DomainEvent::TriggerEscalated`] — fired only on
/// `react`/`escalate`, *in addition* to `TriggerEvaluated`.
pub fn publish_escalated(envelope: &TriggerEnvelope, target_agent: &str) {
    let event = DomainEvent::TriggerEscalated {
        source: envelope.source.slug().to_string(),
        external_id: envelope.external_id.clone(),
        display_label: envelope.display_label.clone(),
        target_agent: target_agent.to_string(),
    };
    record_test_event(&event);
    BUS.publish(event);
}

/// Publish [`DomainEvent::TriggerEscalationFailed`] — fired when the
/// whole pipeline gave up (both local and remote failed, or the
/// classifier reply couldn't be parsed after a retry).
pub fn publish_failed(envelope: &TriggerEnvelope, reason: &str) {
    let event = DomainEvent::TriggerEscalationFailed {
        source: envelope.source.slug().to_string(),
        external_id: envelope.external_id.clone(),
        reason: reason.to_string(),
    };
    record_test_event(&event);
    BUS.publish(event);
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
