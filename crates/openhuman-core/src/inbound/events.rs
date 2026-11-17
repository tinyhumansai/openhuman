//! Inbound events: a trigger as the queues see it, and back.
//!
//! A [`TriggerEnvelope`] becomes an [`InboundEvent`] with
//!
//! - `source` `composio:<toolkit>` or `webhook:<tunnel>`, which the shipped
//!   `pipeline.json` source policies match (`composio:*`, `webhook:*`);
//! - `event_id` the source's own id, so a redelivery has the same
//!   `source:event_id` and the queues reject it. For a Composio trigger that
//!   is its uuid (or id): a trigger the backend redelivers is handled once.
//!   For an agent webhook it is the request's correlation id, which is per
//!   request: a resent request with a new correlation id is a new event;
//! - a `conversation_key` unique to the event, so two triggers are never
//!   coalesced into one turn: each is triaged on its own;
//! - kind `Trigger` (never ambient chatter), and the envelope itself in `raw`,
//!   from which `Deliver` rebuilds it ([`envelope_of`]).
//!
//! An event pushed from an external source (`inbound.offer`) is
//! [`external`]: keyed by the caller's `key`, so a source's events debounce
//! into batches its pinned agent reads back with [`external_of`].
//!
//! A digest turn (the queues folded an overflow into one) becomes a single
//! external envelope listing its events ([`digest_envelope`]), so a flood
//! costs one triage, not one per event.

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tinyeventqueues_bus::{EventKind, InboundEvent, Sender, Turn};

use crate::triage::{TriggerEnvelope, TriggerSource};

/// Source prefix of a Composio trigger.
pub const COMPOSIO: &str = "composio";
/// Source prefix of an agent webhook.
pub const WEBHOOK: &str = "webhook";

/// The largest encoded `raw` envelope that is queued. The storage bus admits
/// 256 KiB per queued message; the event and the queue's framing need the
/// rest. A larger trigger takes the direct path.
pub const MAX_RAW_BYTES: usize = 192 * 1024;

/// How much of a payload the salience text carries.
const TEXT_PAYLOAD_CHARS: usize = 512;

/// The envelope as `raw` stores it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Raw {
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    toolkit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    trigger: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tunnel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    external_id: String,
    display_label: String,
    payload: Value,
    received_at_ms: i64,
}

/// Why an envelope is not queued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unqueueable {
    /// The envelope is not from a source the queues take.
    Source,
    /// The encoded envelope exceeds [`MAX_RAW_BYTES`].
    Oversize(usize),
}

impl std::fmt::Display for Unqueueable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Source => f.write_str("source not queued"),
            Self::Oversize(bytes) => write!(f, "envelope of {bytes} bytes exceeds the queue limit"),
        }
    }
}

/// The salience text: the label and the start of the payload, so similar
/// but distinct triggers do not habituate into noise.
fn text_of(envelope: &TriggerEnvelope) -> String {
    let payload: String = envelope
        .payload
        .to_string()
        .chars()
        .take(TEXT_PAYLOAD_CHARS)
        .collect();
    format!("{} {payload}", envelope.display_label)
}

fn millis(at: DateTime<Utc>) -> u64 {
    u64::try_from(at.timestamp_millis()).unwrap_or(0)
}

/// `id`, or a fresh one when the source gave none (such an event cannot be
/// deduplicated, but it is not lost).
fn id_or_fresh(id: &str) -> String {
    if id.trim().is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        id.to_owned()
    }
}

fn event(
    source: String,
    event_id: String,
    sender: String,
    raw: Raw,
    envelope: &TriggerEnvelope,
) -> Result<InboundEvent, Unqueueable> {
    let raw = serde_json::to_value(&raw).map_err(|_| Unqueueable::Source)?;
    let bytes = raw.to_string().len();
    if bytes > MAX_RAW_BYTES {
        return Err(Unqueueable::Oversize(bytes));
    }
    let conversation = format!("{source}:{event_id}");
    Ok(InboundEvent::new(
        source,
        event_id,
        conversation,
        Sender::new(sender),
        EventKind::Trigger,
        millis(envelope.received_at),
    )
    .text(text_of(envelope))
    .raw(raw))
}

/// The inbound event of a Composio trigger envelope.
///
/// # Errors
///
/// [`Unqueueable::Source`] for a non-Composio envelope,
/// [`Unqueueable::Oversize`] for one too large to queue.
pub fn composio(envelope: &TriggerEnvelope) -> Result<InboundEvent, Unqueueable> {
    let TriggerSource::Composio { toolkit, trigger } = &envelope.source else {
        return Err(Unqueueable::Source);
    };
    let source = format!("{COMPOSIO}:{toolkit}");
    let raw = Raw {
        kind: COMPOSIO.to_owned(),
        toolkit: Some(toolkit.clone()),
        trigger: Some(trigger.clone()),
        tunnel_id: None,
        method: None,
        path: None,
        external_id: envelope.external_id.clone(),
        display_label: envelope.display_label.clone(),
        payload: envelope.payload.clone(),
        received_at_ms: envelope.received_at.timestamp_millis(),
    };
    let event_id = id_or_fresh(&envelope.external_id);
    event(source.clone(), event_id, source, raw, envelope)
}

/// The inbound event of an agent webhook envelope; `correlation_id` is the
/// request's own id (the envelope's external id is the tunnel).
///
/// # Errors
///
/// As [`composio`].
pub fn webhook(
    envelope: &TriggerEnvelope,
    correlation_id: &str,
) -> Result<InboundEvent, Unqueueable> {
    let TriggerSource::Webhook {
        tunnel_id,
        method,
        path,
    } = &envelope.source
    else {
        return Err(Unqueueable::Source);
    };
    let source = format!("{WEBHOOK}:{tunnel_id}");
    let raw = Raw {
        kind: WEBHOOK.to_owned(),
        toolkit: None,
        trigger: None,
        tunnel_id: Some(tunnel_id.clone()),
        method: Some(method.clone()),
        path: Some(path.clone()),
        external_id: envelope.external_id.clone(),
        display_label: envelope.display_label.clone(),
        payload: envelope.payload.clone(),
        received_at_ms: envelope.received_at.timestamp_millis(),
    };
    event(
        source.clone(),
        id_or_fresh(correlation_id),
        source,
        raw,
        envelope,
    )
}

/// Raw kind of an event pushed from an external source (`inbound.offer`).
const EXTERNAL: &str = "external";

/// An external event as `raw` stores it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ExternalRaw {
    kind: String,
    text: String,
    payload: Value,
    received_at_ms: u64,
}

/// One external event as its pinned agent reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalItem {
    /// The source's own event id.
    pub id: String,
    /// The event's text.
    pub text: String,
    /// The source's payload, as pushed.
    pub payload: Value,
    /// When the source received it, Unix epoch milliseconds.
    pub received_at_ms: u64,
}

/// The inbound event of an event pushed from external source `source`
/// (`inbound.offer`). Its `conversation_key` is `key` and its sender the
/// source itself, so the queues debounce a source's events per key into one
/// batch; `event_id` is the source's own id, so a redelivery is rejected.
///
/// # Errors
///
/// [`Unqueueable::Oversize`] for one too large to queue.
pub fn external(
    source: &str,
    key: &str,
    event_id: &str,
    text: &str,
    payload: Value,
    received_at_ms: u64,
) -> Result<InboundEvent, Unqueueable> {
    let raw = serde_json::to_value(ExternalRaw {
        kind: EXTERNAL.to_owned(),
        text: text.to_owned(),
        payload,
        received_at_ms,
    })
    .map_err(|_| Unqueueable::Source)?;
    let bytes = raw.to_string().len();
    if bytes > MAX_RAW_BYTES {
        return Err(Unqueueable::Oversize(bytes));
    }
    Ok(InboundEvent::new(
        source,
        event_id,
        key,
        Sender::new(source),
        EventKind::Trigger,
        received_at_ms,
    )
    .text(text.to_owned())
    .raw(raw))
}

/// The external item an inbound event was offered as, or `None` for an event
/// that was not pushed from an external source.
#[must_use]
pub fn external_of(event: &InboundEvent) -> Option<ExternalItem> {
    let raw: ExternalRaw = serde_json::from_value(event.raw.clone()?).ok()?;
    (raw.kind == EXTERNAL).then(|| ExternalItem {
        id: event.event_id.clone(),
        text: raw.text,
        payload: raw.payload,
        received_at_ms: raw.received_at_ms,
    })
}

/// The envelope an inbound event was queued from, or `None` for an event
/// that did not come from a trigger.
#[must_use]
pub fn envelope_of(event: &InboundEvent) -> Option<TriggerEnvelope> {
    let raw: Raw = serde_json::from_value(event.raw.clone()?).ok()?;
    let source = match raw.kind.as_str() {
        COMPOSIO => TriggerSource::Composio {
            toolkit: raw.toolkit?,
            trigger: raw.trigger?,
        },
        WEBHOOK => TriggerSource::Webhook {
            tunnel_id: raw.tunnel_id?,
            method: raw.method?,
            path: raw.path?,
        },
        _ => return None,
    };
    Some(TriggerEnvelope {
        source,
        external_id: raw.external_id,
        display_label: raw.display_label,
        payload: raw.payload,
        received_at: Utc
            .timestamp_millis_opt(raw.received_at_ms)
            .single()
            .unwrap_or_else(Utc::now),
    })
}

/// One external envelope for a digest turn: the queues folded these events
/// (an overflow, a backlog) so that they are triaged once, together.
#[must_use]
pub fn digest_envelope(turn: &Turn) -> TriggerEnvelope {
    let items: Vec<Value> = turn
        .events
        .iter()
        .map(|event| match envelope_of(event) {
            Some(envelope) => json!({
                "source": event.source,
                "event_id": event.event_id,
                "label": envelope.display_label,
                "payload": envelope.payload,
            }),
            None => json!({
                "source": event.source,
                "event_id": event.event_id,
                "text": event.text,
            }),
        })
        .collect();
    TriggerEnvelope::from_external(
        &format!("inbound-digest/{}", turn.turn_id),
        "inbound_digest",
        json!({
            "kind": "inbound_digest",
            "count": items.len(),
            "events": items,
        }),
    )
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
