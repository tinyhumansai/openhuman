//! Host seams for the runtime turn driver's steering (issue #4456).
//!
//! The turn driver bridges OpenHuman's session-owned [`RunQueue`] into a running
//! turn: it drains queued **steer**/**collect** messages into the run's
//! `SteeringHandle`, forwards mid-flight ones from a poll loop, and cleans up on
//! every exit path (normal return, error, drop-cancellation), requeueing steers
//! the run never applied. The loop is upstream; this module supplies the host
//! seams:
//!
//! - [`QueuedMessage`] for [`QueuedTurn`] (text/id accessors and how a recovered
//!   steer is re-created),
//! - [`queue_feed`], whose event sink publishes
//!   [`DomainEvent::RunQueueMessageDelivered`] /
//!   [`DomainEvent::RunQueueSteerRequeued`] on the event bus,
//! - [`SharedSteeringRegistrar`], which publishes a sub-agent's handle in the
//!   shared steering registry for `steer_subagent` and removes it afterwards.

use std::sync::Arc;

use tinyagents_harness::ids::TaskId;
use tinyagents_harness::run_queue::{ForwardEvent, ForwardEventSink, QueuedMessage, RunQueue};
use tinyagents_harness::steering::SteeringHandle;
use tinyagents_runtime::turn_driver::{RunQueueFeed, SteeringFeed, SteeringRegistrar};

use crate::agent::queued_turn::{text_preview, QueuedTurn};
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;

use super::host::steering::shared_steering_registry;

impl QueuedMessage for QueuedTurn {
    fn id(&self) -> &str {
        &self.id
    }

    fn text(&self) -> &str {
        &self.text
    }

    fn requeued(id: String, text: String, thread_label: &str, queued_at_ms: u64) -> Self {
        QueuedTurn {
            id,
            text,
            client_id: String::new(),
            thread_id: thread_label.to_string(),
            queued_at_ms,
            model_override: None,
            temperature: None,
            locale: None,
        }
    }
}

/// Publish forwarder notifications as `RunQueue*` domain events.
fn event_bus_sink() -> ForwardEventSink {
    Arc::new(|event| match event {
        ForwardEvent::Delivered {
            thread_label,
            mode,
            delivered,
            item_id,
            text,
        } => BUS.publish(DomainEvent::RunQueueMessageDelivered {
            thread_id: thread_label,
            mode: mode.to_string(),
            delivered,
            item_id,
            text_preview: text.as_deref().map(text_preview),
        }),
        ForwardEvent::Requeued {
            thread_label,
            requeued,
            item_id,
            text,
        } => BUS.publish(DomainEvent::RunQueueSteerRequeued {
            thread_id: thread_label,
            requeued,
            item_id,
            text_preview: text.as_deref().map(text_preview),
        }),
    })
}

/// The driver feed over the session's run queue.
pub(super) fn queue_feed(queue: Arc<RunQueue<QueuedTurn>>) -> Arc<dyn SteeringFeed> {
    Arc::new(RunQueueFeed::new(queue, event_bus_sink()))
}

/// Publishes a delegated child's steering handle in the process-wide registry.
pub(super) struct SharedSteeringRegistrar;

impl SteeringRegistrar for SharedSteeringRegistrar {
    fn register(&self, task_id: &str, handle: &SteeringHandle) {
        shared_steering_registry().register(TaskId::new(task_id), handle.clone());
        tracing::debug!(task_id, "[tinyagents] registered subagent steering handle");
    }

    fn deregister(&self, task_id: &str) {
        shared_steering_registry().deregister(&TaskId::new(task_id));
        tracing::debug!(
            task_id,
            "[tinyagents] deregistered subagent steering handle"
        );
    }
}

/// Best-effort thread label for the delivery/requeue observability events and
/// the metadata on any requeued steer: a sub-agent uses its task id; the
/// interactive/channel parent turn reads the turn origin.
pub(super) fn thread_label(
    subagent_scope: Option<&crate::agent::tinyagents::observability::SubagentScope>,
    origin: Option<&crate::agent::turn_origin::AgentTurnOrigin>,
) -> String {
    subagent_scope
        .map(|s| s.task_id.clone())
        .or_else(|| match origin {
            Some(crate::agent::turn_origin::AgentTurnOrigin::WebChat { thread_id, .. }) => {
                Some(thread_id.clone())
            }
            Some(crate::agent::turn_origin::AgentTurnOrigin::ExternalChannel {
                reply_target,
                ..
            }) => Some(reply_target.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "steering_forwarder_tests.rs"]
mod tests;
