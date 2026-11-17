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
//! attempt is ambiguous does the caller fall back to the direct path. The
//! queues may hold that event all the same, so a caller with a direct path
//! offers through [`offer_or_direct`], which settles who runs it
//! ([`super::claims`]): the queues when they delivered it meanwhile, the
//! direct path otherwise, recorded so a later `Deliver` skips it. Callers
//! run the hand-off in their own task, never on a bus subscriber's loop.

use std::sync::Arc;
use std::time::Duration;

use tinybus::module::{ModuleCallError, ModuleClient};
use tinyeventqueues_bus::{Disposition, InboundEvent};

use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::modules::event_queues;

use super::claims::{Claims, DirectClaim};

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

/// How a hand-off ended.
enum HandOff {
    /// Settled: the queues answered, or proved they never saw the event.
    Settled(Offered),
    /// Every attempt was ambiguous: the queues may hold the event.
    Ambiguous(String),
}

/// Offer `event` to the inbound event queues through `client`, for a caller
/// with no direct path of its own: on [`Offered::Direct`] it reports the
/// failure, and an event the queues did hold is delivered once by them.
pub async fn offer(config: &Config, client: &ModuleClient, event: InboundEvent) -> Offered {
    if !super::active(config) {
        return Offered::Direct("inbound event queues inactive".to_owned());
    }
    match hand_off(client, event, &ATTEMPT_DEADLINES).await {
        HandOff::Settled(offered) => offered,
        HandOff::Ambiguous(why) => Offered::Direct(why),
    }
}

/// Offer `event` like [`offer`], for a caller that runs its direct path on
/// [`Offered::Direct`]. When every attempt is ambiguous the event goes to
/// the direct path only if the queues have not delivered it and no other
/// offer of it took the direct path; the [`DirectClaim`] then returned is
/// the caller's to finish with what its direct path did, so the queues'
/// copy is dropped or delivered accordingly.
pub async fn offer_or_direct(
    config: &Config,
    client: &ModuleClient,
    event: InboundEvent,
) -> (Offered, Option<DirectClaim>) {
    if !super::active(config) {
        return (
            Offered::Direct("inbound event queues inactive".to_owned()),
            None,
        );
    }
    let claims = Claims::of_workspace(&config.workspace_dir, chrono::Utc::now().timestamp_millis());
    offer_or_direct_with(client, event, &claims, &ATTEMPT_DEADLINES).await
}

async fn offer_or_direct_with(
    client: &ModuleClient,
    event: InboundEvent,
    claims: &Arc<Claims>,
    deadlines: &[Duration],
) -> (Offered, Option<DirectClaim>) {
    let key = event.dedupe_key();
    claims.open(&key);
    match hand_off(client, event, deadlines).await {
        HandOff::Settled(Offered::Direct(why)) => {
            // The queues never saw this offer. Alone, the direct path needs
            // no claim; another offer of the event may have reached them,
            // though, so then it goes through the claims like an ambiguous one.
            if claims.close(&key) {
                (Offered::Direct(why), None)
            } else {
                take_direct(claims, &key, why)
            }
        }
        HandOff::Settled(offered) => {
            claims.close(&key);
            (offered, None)
        }
        HandOff::Ambiguous(why) => take_direct(claims, &key, why),
    }
}

/// The direct path for `key`, when the claims grant it.
fn take_direct(claims: &Arc<Claims>, key: &str, why: String) -> (Offered, Option<DirectClaim>) {
    match claims.take_direct(key) {
        Some(claim) => (Offered::Direct(why), Some(claim)),
        None => {
            tracing::info!(
                key = %key,
                "[inbound] the queues or another offer have the event; not taking the direct path"
            );
            (Offered::Queued(Disposition::Duplicate), None)
        }
    }
}

async fn hand_off(client: &ModuleClient, event: InboundEvent, deadlines: &[Duration]) -> HandOff {
    let source = event.source.clone();
    let event_id = event.event_id.clone();
    let mut last = String::new();
    let mut answered = None;
    for (n, &deadline) in deadlines.iter().enumerate() {
        if n > 0 {
            tokio::time::sleep(RESEND_PAUSE).await;
        }
        match attempt(client, &event, deadline).await {
            Attempt::Answered(response) => {
                answered = Some(response);
                break;
            }
            Attempt::Unsent(error) if n == 0 => {
                tracing::info!(
                    source = %source,
                    event_id = %event_id,
                    error = %error,
                    "[inbound] queues unavailable; taking the direct path"
                );
                return HandOff::Settled(Offered::Direct(format!("ingest failed: {error}")));
            }
            Attempt::Unsent(error) => {
                // This attempt never reached the module, but an earlier one
                // may have: the hand-off stays ambiguous.
                tracing::warn!(
                    source = %source,
                    event_id = %event_id,
                    attempt = n + 1,
                    error = %error,
                    "[inbound] queues unavailable after an unanswered ingest; the queues may hold the event"
                );
                return HandOff::Ambiguous(format!("ingest failed: {last}; then {error}"));
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
            "[inbound] ingest never answered; the queues may hold the event"
        );
        return HandOff::Ambiguous(format!("ingest failed: {last}"));
    };
    let Some(result) = response.results.into_iter().next() else {
        tracing::warn!(source = %source, "[inbound] ingest answered no result; taking the direct path");
        return HandOff::Settled(Offered::Direct("ingest answered no result".to_owned()));
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
    HandOff::Settled(Offered::Queued(result.disposition))
}

#[cfg(test)]
#[path = "route_tests.rs"]
mod tests;
