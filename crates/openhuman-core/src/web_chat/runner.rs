//! The web chat's turn runner and the host state its ports share.
//!
//! TinyAgents' [`TurnRunner`] owns the lifecycle of a turn: ingress order,
//! per-thread serialization, the queue modes, cancellation generations, the
//! idle and wall-clock guards, reply persistence and exactly one terminal
//! event. This host supplies the ports: session checkout and the agent run
//! ([`turn_host`](super::turn_host)), prompt policy at the door
//! ([`ingress`](super::ingress)), and the delivery, failure copy and grounded
//! close ([`turn_events`](super::turn_events)). The ports share the per-request
//! state kept here.
//!
//! One [`WebRuntime`] exists per core context (an embedded agent, a SaaS
//! profile or the host's own), the way the thread-session cache does.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tinyagents_harness::turn_runner::{ScopeKey, TurnRunner, TurnRunnerConfig};
use tokio::sync::{oneshot, Mutex as AsyncMutex};

use super::ingress::WebIngress;
use super::ops::StartChatError;
use super::progress::ProgressProjection;
use super::session::CheckedOutSession;
use super::turn_activity::TurnActivity;
use super::turn_events::{WebEvents, WebFailures, WebReplies, WebSummarizer};
use super::turn_host::WebHost;
use super::types::WebChatTaskResult;
use crate::agent::queued_turn::QueuedPayload;
use crate::agent::session_lease::SessionLease;
use crate::core::runtime::DispatchCtx;

pub(super) type WebRunner = TurnRunner<
    WebHost,
    WebIngress,
    WebReplies,
    WebEvents,
    WebFailures,
    WebSummarizer,
    QueuedPayload,
>;

/// How long a stopped turn may keep running so the session runtime can persist
/// the rounds it completed. Just above the runtime's own 2 s driver grace; the
/// user-facing `chat_cancelled` has already been decided.
const STOPPED_TURN_PERSIST_GRACE: Duration = Duration::from_secs(3);

/// Upper bound on closing a stopped turn: config, snapshot read and the
/// closing model call (itself bounded at 45s). Past it the stop is reported as
/// a `turn_timeout` error, so closing can never be what hangs the turn.
const CLOSE_REPLY_BUDGET: Duration = Duration::from_secs(60);

/// What `start_chat` learns about a submission before it returns.
#[derive(Debug)]
pub(super) enum Admission {
    /// The turn is accepted and runs (or is about to).
    Started,
    /// The thread has a turn in flight and the message joined its queue.
    Queued,
    /// An approval reply the gate consumed; carries the approval request id.
    Consumed(String),
    /// Refused at the door.
    Rejected(StartChatError),
}

/// Why a turn was cancelled, for the events its terminal publishes.
#[derive(Clone, Debug, Default)]
pub(super) struct CancelNote {
    /// `user_stop` for a Stop; the default is a newer request superseding it.
    pub(super) user_stop: bool,
    /// The client to address the events to, when it is not the one that
    /// started the turn (a Stop from another client).
    pub(super) client_id: Option<String>,
    pub(super) superseded_by: Option<String>,
}

/// Everything the ports know about one request.
#[derive(Default)]
pub(super) struct TurnSlot {
    pub(super) activity: TurnActivity,
    pub(super) session: Option<CheckedOutSession>,
    /// Cross-surface lease survives TinyAgents' early `checkin` until the
    /// terminal projection has finished delivering this turn.
    pub(super) session_lease: Option<SessionLease>,
    /// Profile-wide serialization is acquired after the session lease (the
    /// same order used by native channel turns) and held through terminal
    /// projection. Acquiring these in the opposite order can deadlock when
    /// two Runner instances target one session.
    pub(super) profile_turn: Option<crate::profiles::turn_gate::TurnGuard>,
    pub(super) fork: bool,
    pub(super) config: Option<Arc<crate::config::Config>>,
    pub(super) thread_dir: Option<std::path::PathBuf>,
    pub(super) projection: Option<Arc<AsyncMutex<ProgressProjection>>>,
    pub(super) result: Option<WebChatTaskResult>,
    pub(super) admission: Option<oneshot::Sender<Admission>>,
    pub(super) cancel: Option<CancelNote>,
}

/// Per-request state shared by the runner's ports.
#[derive(Default)]
pub(super) struct Shared {
    turns: Mutex<HashMap<String, TurnSlot>>,
    /// Requests that joined a run queue and have not started yet.
    queued: Mutex<HashSet<String>>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Shared {
    /// Run `f` on the slot of `request_id`, creating it on first use.
    pub(super) fn with<R>(&self, request_id: &str, f: impl FnOnce(&mut TurnSlot) -> R) -> R {
        f(locked(&self.turns)
            .entry(request_id.to_owned())
            .or_default())
    }

    /// Drop the slot of a finished request and hand back what it held.
    pub(super) fn take(&self, request_id: &str) -> Option<TurnSlot> {
        locked(&self.turns).remove(request_id)
    }

    /// Tell the submitter how its message was received; only the first
    /// outcome counts.
    pub(super) fn admit(&self, request_id: &str, admission: Admission) {
        let sender = locked(&self.turns)
            .get_mut(request_id)
            .and_then(|slot| slot.admission.take());
        if let Some(sender) = sender {
            let _ = sender.send(admission);
        }
    }

    pub(super) fn mark_queued(&self, request_id: &str) {
        locked(&self.queued).insert(request_id.to_owned());
        // A queued message keeps no turn state until it runs.
        self.take(request_id);
    }

    /// Whether `request_id` waited in a run queue; clears the mark.
    pub(super) fn was_queued(&self, request_id: &str) -> bool {
        locked(&self.queued).remove(request_id)
    }

    pub(super) fn forget_queued(&self, request_id: &str) {
        locked(&self.queued).remove(request_id);
    }

    pub(super) fn note_cancel(&self, request_id: &str, note: CancelNote) {
        self.with(request_id, |slot| slot.cancel = Some(note));
    }
}

/// The runner and the state its ports share, for one core context.
pub(super) struct WebRuntime {
    pub(super) shared: Arc<Shared>,
    pub(super) runner: Arc<WebRunner>,
}

impl Default for WebRuntime {
    fn default() -> Self {
        let shared = Arc::new(Shared::default());
        let ingress = Arc::new(WebIngress::new(shared.clone()));
        let runner = Arc::new(TurnRunner::new(
            Arc::new(WebHost::new(shared.clone())),
            ingress.clone(),
            Arc::new(WebReplies::new(shared.clone())),
            Arc::new(WebEvents::new(shared.clone())),
            Arc::new(WebFailures),
            Arc::new(WebSummarizer::new(shared.clone())),
            TurnRunnerConfig {
                close_timeout: CLOSE_REPLY_BUDGET,
                cancel_grace: STOPPED_TURN_PERSIST_GRACE,
                ..TurnRunnerConfig::default()
            },
        ));
        ingress.bind(&runner);
        Self { shared, runner }
    }
}

/// The runtime of the core context `ctx` runs under.
pub(super) fn runtime(ctx: &DispatchCtx) -> Arc<WebRuntime> {
    crate::core::runtime::slot_for::<WebRuntime>(ctx)
}

/// The scope a thread's turns serialize under for the calling context.
pub(super) fn scope_of(ctx: &DispatchCtx) -> ScopeKey {
    let tenant = super::ops::tenant_of(ctx);
    ScopeKey {
        agent_id: tenant.agent.unwrap_or_default(),
        profile_id: tenant.profile,
    }
}
