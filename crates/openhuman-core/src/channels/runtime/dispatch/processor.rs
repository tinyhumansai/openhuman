//! Core message processing loop for the channel runtime.
//!
//! Contains:
//! * [`channel_has_approval_surface`] — gate controlling per-channel approval
//!   context scoping.
//! * [`try_route_approval_reply`] (in [`approval`]) — intercepts yes/no
//!   approval replies before dispatching a fresh agent turn.
//! * [`process_channel_message`] — full per-message pipeline: typing, ACK
//!   reaction, durable session, agent turn, progressive events and reply.
//! * [`bus_turn`] — the orchestrator path: an unbound channel's turn over
//!   the native bus. A channel bound to a host agent runs through
//!   [`super::host_agent`] instead.
//! * [`admit`] — preparation, the received event, and the runtime-command and
//!   approval-reply intercepts that run before any debounce.
//! * [`run_message_dispatch_loop`] — admits inbound messages, debounces them
//!   per conversation through [`crate::inbound_debounce`], and starts one turn
//!   per batch with a bounded set of host workers.

mod admit;
mod approval;
mod bus_turn;
mod in_order;
mod native_media;
pub(crate) mod progressive;
mod reset;
mod turn;

#[cfg(test)]
pub(crate) use native_media::drop_media_read_lease_for_test;

use super::inbound_batch::{merge_channel_batch, QueuedInbound};
use crate::channels::context::ChannelRuntimeContext;
use crate::config::InboundDebounceConfig;
use crate::inbound_debounce::KeyedDebouncer;
use in_order::InOrder;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tinychannels_bus::{ChannelInboundEnvelope, ChannelMessage};
use tokio::task::JoinSet;
use tokio::time::Instant;

/// Host-side inbound work item. Relay authentication remains out-of-band and
/// can only be set by the native callback after receiving the trusted module
/// callback; it is never reconstructed from envelope metadata.
#[derive(Clone)]
pub(crate) struct RuntimeChannelMessage {
    pub(crate) message: ChannelMessage,
    pub(crate) inbound_envelope: Option<ChannelInboundEnvelope>,
    pub(crate) authenticated_relay: bool,
    pub(crate) prepared_inbound: Option<tinychannels_bus::module_inbound::PreparedChannelInbound>,
    pub(crate) imported_history: Vec<tinyagents_session::transcript::TranscriptMessage>,
    pub(crate) legacy_history: Option<crate::channels::providers::relay::LegacyHistoryImport>,
    /// Trusted per-turn callback target installed by a hosted relay. Native
    /// configured channels resolve their target from the prepared envelope.
    pub(crate) progressive_target: Option<Arc<progressive::NativeProgressiveTarget>>,
}

impl RuntimeChannelMessage {
    pub(crate) fn with_inbound_envelope(
        message: ChannelMessage,
        inbound_envelope: ChannelInboundEnvelope,
    ) -> Self {
        Self {
            message,
            inbound_envelope: Some(inbound_envelope),
            authenticated_relay: false,
            prepared_inbound: None,
            imported_history: Vec::new(),
            legacy_history: None,
            progressive_target: None,
        }
    }

    pub(crate) fn from_authenticated_relay(
        delivery: tinychannels_bus::module_relay::RelayInboundDelivery,
    ) -> Self {
        Self {
            // The module's PrepareChannelInbound operation supplies the
            // normalized legacy projection before the processor consumes it.
            message: ChannelMessage::default(),
            inbound_envelope: Some(delivery.envelope),
            authenticated_relay: delivery.delivered_via_authenticated_relay,
            prepared_inbound: None,
            imported_history: Vec::new(),
            legacy_history: None,
            progressive_target: None,
        }
    }
}

impl From<ChannelMessage> for RuntimeChannelMessage {
    fn from(message: ChannelMessage) -> Self {
        Self {
            message,
            inbound_envelope: None,
            authenticated_relay: false,
            prepared_inbound: None,
            imported_history: Vec::new(),
            legacy_history: None,
            progressive_target: None,
        }
    }
}

use admit::admit_runtime_message;
pub(crate) use admit::AdmittedInbound;
#[cfg(test)]
pub(crate) use approval::channel_has_approval_surface;
pub(crate) use reset::reset_durable_channel_session;
pub(crate) use turn::process_channel_runtime_message;
use turn::run_admitted_turn;

pub(crate) async fn process_channel_message(ctx: Arc<ChannelRuntimeContext>, msg: ChannelMessage) {
    process_channel_runtime_message(ctx, RuntimeChannelMessage::from(msg)).await;
}

/// Run every inbound message through the agent pipeline until `rx` closes.
///
/// Each message is admitted first (preparation, the received event, runtime
/// commands and approval replies), concurrently but released in receive order.
/// Admitted messages are then debounced per conversation (`history_key`): a
/// burst becomes one turn. The debouncer only delays and merges intake. It
/// releases a conversation as soon as its batch is handed to the shared
/// TinyAgents turn runner, which still owns queueing and steering for
/// messages that arrive while a turn runs. At most `max_in_flight_messages`
/// turns start concurrently, and the loop stops receiving while
/// [`intake_budget`] admitted messages are waiting for a turn, so a sustained
/// sender is held back by the bounded inbound queue.
pub(crate) async fn run_message_dispatch_loop(
    rx: tokio::sync::mpsc::Receiver<RuntimeChannelMessage>,
    ctx: Arc<ChannelRuntimeContext>,
    max_in_flight_messages: usize,
    debounce: InboundDebounceConfig,
) {
    let max_in_flight = max_in_flight_messages.max(1);
    let budget = intake_budget(max_in_flight, &debounce);
    let mut held: usize = 0;
    let mut rx = rx;
    let mut inbound_open = true;
    let mut next_seq: u64 = 0;
    let mut admitting = JoinSet::new();
    let mut admit_seqs: HashMap<tokio::task::Id, u64> = HashMap::new();
    let mut admission_order = InOrder::new();
    let mut turns = JoinSet::new();
    let mut handoffs: JoinSet<String> = JoinSet::new();
    let mut debouncer: KeyedDebouncer<String, QueuedInbound> = KeyedDebouncer::new();
    let mut ready: VecDeque<(String, Vec<QueuedInbound>)> = VecDeque::new();

    loop {
        while turns.len() < max_in_flight {
            let Some((key, batch)) = ready.pop_front() else {
                break;
            };
            let batch_len = batch.len();
            held = held.saturating_sub(batch_len);
            match merge_channel_batch(batch) {
                None => debouncer.complete(&key),
                Some(admitted) => {
                    tracing::debug!(
                        channel = %admitted.msg.channel,
                        message_id = %admitted.msg.id,
                        batch_len,
                        in_flight = turns.len() + 1,
                        held,
                        "[dispatch][debounce] starting turn"
                    );
                    // The turn runner owns serialization and steering per
                    // conversation. The debouncer keeps a key only until its batch
                    // is handed to the runner, so the next batch of that
                    // conversation is handed over after it, in receive order.
                    let (handoff, handed_off) = tokio::sync::oneshot::channel::<()>();
                    turns.spawn(run_admitted_turn(Arc::clone(&ctx), admitted, Some(handoff)));
                    handoffs.spawn(async move {
                        let _ = handed_off.await;
                        key
                    });
                }
            }
        }
        if !inbound_open
            && admitting.is_empty()
            && turns.is_empty()
            && handoffs.is_empty()
            && ready.is_empty()
            && debouncer.is_empty()
        {
            break;
        }

        let wakeup = debouncer.next_wakeup();
        tokio::select! {
            received = rx.recv(), if inbound_open
                && admitting.len() < max_in_flight
                && admitting.len() + admission_order.waiting() + held < budget => {
                match received {
                    Some(message) => {
                        tracing::debug!(channel = %message.message.channel, message_id = %message.message.id, "[dispatch] received module inbound message");
                        let seq = next_seq;
                        next_seq += 1;
                        let handle = admitting.spawn(admit_runtime_message(Arc::clone(&ctx), message));
                        admit_seqs.insert(handle.id(), seq);
                    }
                    None => {
                        tracing::debug!(
                            buffered_keys = debouncer.tracked(),
                            "[dispatch][debounce] inbound queue closed; flushing buffered messages"
                        );
                        inbound_open = false;
                    }
                }
            }
            Some(joined) = admitting.join_next_with_id(), if !admitting.is_empty() => {
                let (id, result) = match joined {
                    Ok((id, result)) => (id, result),
                    Err(error) => {
                        let id = error.id();
                        log_join_error("admission", Some(error));
                        (id, None)
                    }
                };
                if let Some(seq) = admit_seqs.remove(&id) {
                    for (seq, inbound) in admission_order.finish(seq, result) {
                        held += 1;
                        buffer_admitted(&debounce, &mut debouncer, seq, inbound);
                    }
                }
            }
            Some(joined) = handoffs.join_next(), if !handoffs.is_empty() => {
                match joined {
                    Ok(key) => debouncer.complete(&key),
                    Err(error) => log_join_error("handoff", Some(error)),
                }
            }
            Some(joined) = turns.join_next(), if !turns.is_empty() => {
                if let Err(error) = joined {
                    log_join_error("turn", Some(error));
                }
            }
            _ = tokio::time::sleep_until(wakeup.unwrap_or_else(Instant::now)), if wakeup.is_some() => {}
        }

        let now = Instant::now();
        if !inbound_open && admitting.is_empty() {
            debouncer.flush_all(now);
        }
        ready.extend(debouncer.poll_ready(now));
    }
}

/// Admitted messages the loop holds before it stops receiving: a full batch
/// for every turn worker.
fn intake_budget(max_in_flight: usize, debounce: &InboundDebounceConfig) -> usize {
    max_in_flight.saturating_mul(debounce.max_batch_messages.max(1))
}

fn buffer_admitted(
    debounce: &InboundDebounceConfig,
    debouncer: &mut KeyedDebouncer<String, QueuedInbound>,
    seq: u64,
    inbound: AdmittedInbound,
) {
    let settings = debounce.settings_for(&inbound.msg.channel);
    let channel = inbound.msg.channel.clone();
    let key = inbound.history_key.clone();
    let push = debouncer.push(
        key,
        QueuedInbound { seq, inbound },
        &settings,
        Instant::now(),
    );
    tracing::debug!(
        channel = %channel,
        seq,
        outcome = ?push,
        tracked = debouncer.tracked(),
        "[dispatch][debounce] buffered inbound message"
    );
}

fn log_join_error(stage: &str, error: Option<tokio::task::JoinError>) {
    if let Some(error) = error {
        tracing::warn!("channel dispatch {stage} worker failed: {error}");
    }
}
