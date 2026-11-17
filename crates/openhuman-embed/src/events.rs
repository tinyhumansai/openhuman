//! Content-free runtime observations with bounded, explicit backpressure.

use futures_core::Stream;
use openhuman_core::core::events::DomainEvent;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll};
use tokio::sync::broadcast;
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream};
use {openhuman_core::core::bus::BUS, tinybus::EventHandler, tinybus::SubscriptionHandle};

/// Metadata about one runtime transition; never includes user/tool payloads.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeEvent {
    /// Monotonically increasing sequence within this runtime.
    pub sequence: u64,
    /// The agent responsible for the transition, when one is known.
    pub agent_id: Option<String>,
    /// Unique id for this call, separate from its durable conversation id.
    pub turn_id: Option<String>,
    /// The transition being observed.
    pub kind: RuntimeEventKind,
}

/// Content-free transitions published by the embedding runtime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeEventKind {
    /// An agent was registered.
    AgentAdded,
    /// An agent was removed or its final handle dropped.
    AgentRemoved,
    /// A host turn started.
    TurnStarted,
    /// A host turn ended, including errors and cancellation.
    TurnEnded {
        /// Whether the turn returned a successful outcome.
        success: bool,
    },
    /// A tool started, without its arguments.
    ToolStarted {
        /// The declared tool name.
        tool_name: String,
    },
    /// A tool ended, without its result.
    ToolEnded {
        /// The declared tool name.
        tool_name: String,
        /// Whether execution succeeded.
        success: bool,
    },
    /// A tool is waiting for the host's permission.
    ApprovalRequested {
        /// Opaque approval request identifier.
        request_id: String,
    },
    /// A pending permission was decided.
    ApprovalDecided {
        /// Opaque approval request identifier.
        request_id: String,
        /// Stable decision classification.
        decision: String,
    },
    /// Background services were started or stopped.
    ServicesChanged {
        /// Whether the selected services are running.
        running: bool,
    },
}

/// A subscriber fell behind the runtime's bounded event buffer.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EventStreamError {
    /// These events were overwritten; resynchronise using runtime inspection.
    #[error("runtime event subscriber missed {0} events")]
    Lagged(u64),
    /// The runtime and all of its agents have been dropped.
    #[error("runtime event stream closed")]
    Closed,
}

/// A runtime event subscription; lag is reported rather than hidden.
pub struct RuntimeEvents {
    inner: BroadcastStream<RuntimeEvent>,
}
impl RuntimeEvents {
    /// Receive the next event, or an explicit lag/closure error.
    pub async fn recv(&mut self) -> Result<RuntimeEvent, EventStreamError> {
        std::future::poll_fn(|cx| Pin::new(&mut *self).poll_next(cx))
            .await
            .unwrap_or(Err(EventStreamError::Closed))
    }
}
impl Stream for RuntimeEvents {
    type Item = Result<RuntimeEvent, EventStreamError>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx).map(|item| {
            item.map(|result| {
                result.map_err(|BroadcastStreamRecvError::Lagged(n)| EventStreamError::Lagged(n))
            })
        })
    }
}

#[derive(Default)]
struct HubState {
    sequence: u64,
    agents: BTreeSet<String>,
    active: BTreeMap<(String, String), BTreeSet<String>>,
    // Retain correlation until the decision arrives, even if the turn already
    // ended: the core bus delivers events asynchronously.
    approvals: BTreeMap<String, (String, Option<String>)>,
}

pub(crate) struct EventHub {
    sender: broadcast::Sender<RuntimeEvent>,
    state: Mutex<HubState>,
    subscription: Mutex<Option<SubscriptionHandle>>,
}
impl EventHub {
    pub(crate) fn new(capacity: usize) -> Arc<Self> {
        let (sender, _) = broadcast::channel(capacity.max(1));
        let hub = Arc::new(Self {
            sender,
            state: Mutex::new(HubState::default()),
            subscription: Mutex::new(None),
        });
        *hub.subscription
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            BUS.subscribe(Arc::new(ApprovalEvents(Arc::downgrade(&hub))));
        hub
    }
    pub(crate) fn subscribe(&self) -> RuntimeEvents {
        RuntimeEvents {
            inner: BroadcastStream::new(self.sender.subscribe()),
        }
    }
    fn publish(
        &self,
        state: &mut HubState,
        agent_id: Option<String>,
        turn_id: Option<String>,
        kind: RuntimeEventKind,
    ) {
        state.sequence += 1;
        let _ = self.sender.send(RuntimeEvent {
            sequence: state.sequence,
            agent_id,
            turn_id,
            kind,
        });
    }
    pub(crate) fn emit(
        &self,
        agent_id: Option<String>,
        turn_id: Option<String>,
        kind: RuntimeEventKind,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(agent) = &agent_id {
            match kind {
                RuntimeEventKind::AgentAdded => {
                    state.agents.insert(agent.clone());
                }
                RuntimeEventKind::AgentRemoved => {
                    state.agents.remove(agent);
                    state.active.retain(|(owner, _), _| owner != agent);
                    state.approvals.retain(|_, (owner, _)| owner != agent);
                }
                _ => {}
            }
        }
        self.publish(&mut state, agent_id, turn_id, kind);
    }
    pub(crate) fn begin_turn(&self, agent_id: Option<String>, thread_id: &str) -> String {
        let turn_id = format!("turn-{}", uuid::Uuid::new_v4());
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(agent) = &agent_id {
            state
                .active
                .entry((agent.clone(), thread_id.to_owned()))
                .or_default()
                .insert(turn_id.clone());
        }
        self.publish(
            &mut state,
            agent_id,
            Some(turn_id.clone()),
            RuntimeEventKind::TurnStarted,
        );
        turn_id
    }
    pub(crate) fn end_turn(
        &self,
        agent_id: Option<String>,
        thread_id: &str,
        turn_id: &str,
        success: bool,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(agent) = &agent_id {
            let key = (agent.clone(), thread_id.to_owned());
            if let Some(turns) = state.active.get_mut(&key) {
                turns.remove(turn_id);
                if turns.is_empty() {
                    state.active.remove(&key);
                }
            }
        }
        self.publish(
            &mut state,
            agent_id,
            Some(turn_id.to_owned()),
            RuntimeEventKind::TurnEnded { success },
        );
    }
    fn approval(
        &self,
        agent_id: &Option<String>,
        thread_id: &Option<String>,
        request_id: &str,
        decision: Option<&str>,
    ) {
        let Some(agent) = agent_id else {
            return;
        };
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.agents.contains(agent) {
            return;
        }
        let active = thread_id
            .as_ref()
            .and_then(|thread| state.active.get(&(agent.clone(), thread.clone())))
            .and_then(|turns| {
                // The wire event names a conversation, not a call. Concurrent calls
                // in one conversation cannot be distinguished safely.
                (turns.len() == 1)
                    .then(|| turns.iter().next().cloned())
                    .flatten()
            });
        let (turn_id, kind) = match decision {
            Some(decision) => {
                let turn = state
                    .approvals
                    .remove(request_id)
                    .filter(|(owner, _)| owner == agent)
                    .map(|(_, turn)| turn)
                    .unwrap_or(active);
                (
                    turn,
                    RuntimeEventKind::ApprovalDecided {
                        request_id: request_id.to_owned(),
                        decision: decision.to_owned(),
                    },
                )
            }
            None => {
                state
                    .approvals
                    .insert(request_id.to_owned(), (agent.clone(), active.clone()));
                (
                    active,
                    RuntimeEventKind::ApprovalRequested {
                        request_id: request_id.to_owned(),
                    },
                )
            }
        };
        self.publish(&mut state, Some(agent.clone()), turn_id, kind);
    }
}
struct ApprovalEvents(Weak<EventHub>);
#[async_trait::async_trait]
impl EventHandler<DomainEvent> for ApprovalEvents {
    fn name(&self) -> &str {
        "embed.runtime.approvals"
    }
    async fn handle(&self, event: &DomainEvent) {
        let Some(hub) = self.0.upgrade() else {
            return;
        };
        match event {
            DomainEvent::ApprovalRequested {
                request_id,
                agent_id,
                thread_id,
                ..
            } => hub.approval(agent_id, thread_id, request_id, None),
            DomainEvent::ApprovalDecided {
                request_id,
                decision,
                agent_id,
                thread_id,
                ..
            } => hub.approval(agent_id, thread_id, request_id, Some(decision)),
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
