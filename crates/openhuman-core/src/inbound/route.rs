//! Handing an inbound event to the queues.
//!
//! [`offer`] is what an inbound path calls instead of running triage itself.
//! When the queues are active and take the event, the path is done: the
//! module persists it before it answers, so the platform may be acknowledged,
//! and it calls the host back (`Deliver`) when the event is due. When they
//! are inactive or unavailable (the module cannot load on this host, the call
//! fails, the event is too large to queue), the caller runs its direct path,
//! exactly as it did before the queues existed. Nothing is dropped by the
//! hand-off itself.
//!
//! Every attempt has a deadline, and the first is long enough for the module's
//! first load (a release download). A failure that proves the module never saw
//! the event (unavailable, another contract) takes the direct path at once. A
//! failure that does not (a timeout, a transport fault) sends the event again:
//! `Ingest` is idempotent on `source:event_id`, so an event the module did
//! persist comes back `Duplicate` rather than queued twice. Only when every
//! attempt is ambiguous does the caller fall back to the direct path, and
//! that one case can triage an event the queues also deliver; it is logged.
//! Callers run the hand-off in their own task, never on a bus subscriber's
//! loop.

use std::time::Duration;

use tinybus::module::{ModuleCallError, ModuleClient};
use tinyeventqueues_bus::{Disposition, InboundEvent};

use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::modules::event_queues;

/// What became of an offered event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offered {
    /// The queues own the event now: accepted, or ended at ingress (a
    /// redelivery of an event already taken, an invalid envelope). The caller
    /// does nothing more.
    Queued(Disposition),
    /// The queues did not take it; the caller runs its direct path.
    Direct(String),
}

impl Offered {
    /// Whether the queues own the event.
    #[must_use]
    pub fn is_queued(&self) -> bool {
        matches!(self, Self::Queued(_))
    }
}

/// Deadlines of the successive `Ingest` attempts: the first covers the
/// module's first load.
const ATTEMPT_DEADLINES: [Duration; 3] = [
    Duration::from_secs(60),
    Duration::from_secs(10),
    Duration::from_secs(10),
];

/// Pause before re-sending after an ambiguous failure.
const RESEND_PAUSE: Duration = Duration::from_millis(500);

/// How one `Ingest` attempt ended.
enum Attempt {
    Answered(tinyeventqueues_bus::IngestResponse),
    /// The module never received the event.
    Unsent(String),
    /// The module may have persisted the event.
    Unknown(String),
}

async fn attempt(client: &ModuleClient, event: &InboundEvent, deadline: Duration) -> Attempt {
    match tokio::time::timeout(deadline, event_queues::ingest(client, vec![event.clone()])).await {
        Ok(Ok(response)) => Attempt::Answered(response),
        Ok(Err(error @ (ModuleCallError::Unavailable | ModuleCallError::IncompatibleContract))) => {
            Attempt::Unsent(error.to_string())
        }
        Ok(Err(error)) => Attempt::Unknown(error.to_string()),
        Err(_) => Attempt::Unknown(format!("no answer within {deadline:?}")),
    }
}

/// Offer `event` to the inbound event queues through `client`.
pub async fn offer(config: &Config, client: &ModuleClient, event: InboundEvent) -> Offered {
    if !super::active(config) {
        return Offered::Direct("inbound event queues inactive".to_owned());
    }
    let source = event.source.clone();
    let event_id = event.event_id.clone();
    let mut last = String::new();
    let mut answered = None;
    for (n, deadline) in ATTEMPT_DEADLINES.into_iter().enumerate() {
        if n > 0 {
            tokio::time::sleep(RESEND_PAUSE).await;
        }
        match attempt(client, &event, deadline).await {
            Attempt::Answered(response) => {
                answered = Some(response);
                break;
            }
            Attempt::Unsent(error) => {
                tracing::info!(
                    source = %source,
                    event_id = %event_id,
                    error = %error,
                    "[inbound] queues unavailable; taking the direct path"
                );
                return Offered::Direct(format!("ingest failed: {error}"));
            }
            Attempt::Unknown(error) => {
                tracing::warn!(
                    source = %source,
                    event_id = %event_id,
                    attempt = n + 1,
                    error = %error,
                    "[inbound] ingest answer lost; sending again"
                );
                last = error;
            }
        }
    }
    let Some(response) = answered else {
        tracing::warn!(
            source = %source,
            event_id = %event_id,
            error = %last,
            "[inbound] ingest never answered; taking the direct path (the queues may also deliver it)"
        );
        return Offered::Direct(format!("ingest failed: {last}"));
    };
    let Some(result) = response.results.into_iter().next() else {
        tracing::warn!(source = %source, "[inbound] ingest answered no result; taking the direct path");
        return Offered::Direct("ingest answered no result".to_owned());
    };
    let reason = result
        .reason
        .and_then(|reason| serde_json::to_value(reason).ok())
        .and_then(|value| value.as_str().map(str::to_owned));
    match result.disposition {
        Disposition::Accepted => {
            tracing::debug!(source = %source, event_id = %event_id, "[inbound] queued");
        }
        disposition => {
            let reason = reason.unwrap_or_else(|| format!("{disposition:?}").to_lowercase());
            tracing::info!(
                source = %source,
                event_id = %event_id,
                reason = %reason,
                "[inbound] ended at ingress"
            );
            BUS.publish(DomainEvent::InboundEventDropped {
                source,
                event_id,
                reason,
            });
        }
    }
    Offered::Queued(result.disposition)
}

#[cfg(test)]
#[path = "route_tests.rs"]
mod tests;
