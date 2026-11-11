//! Owner-scoped run sessions and the single monotonic PollRun reader.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use tinyflows::engine::GraphObservation;
use tinyflows_bus::catalog::Workflow as Flow;
use tinyflows_bus::runs::FlowRunTrigger;
use tinyflows_bus::runs::{self, RunEvent};

use super::{projection, RunBindings};
use crate::config::Config;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RunKey {
    destination: String,
    object_path: String,
    run_id: String,
}

impl RunKey {
    pub(crate) fn new(storage: &tinyflows_bus::catalog::CatalogStorage, run_id: &str) -> Self {
        Self {
            destination: storage.callback.destination.clone(),
            object_path: storage.callback.object_path.clone(),
            run_id: run_id.to_owned(),
        }
    }
}

/// One live or parked run session, bound to a stable owner address and one
/// module event cursor. The poller is the only reader that advances this cursor;
/// synchronous callers receive copies through independent channels.
pub(crate) struct RunSession {
    pub(super) bindings: std::sync::RwLock<Arc<RunBindings>>,
    pub(super) config: Arc<Config>,
    pub(super) flow: tokio::sync::RwLock<Flow>,
    flow_id: String,
    pub(super) trigger: Mutex<FlowRunTrigger>,
    pub(super) run_id: String,
    cursor: std::sync::atomic::AtomicU64,
    poll_lock: tokio::sync::Mutex<()>,
    watchers: Mutex<Vec<tokio::sync::mpsc::UnboundedSender<RunEvent>>>,
    poller_started: std::sync::atomic::AtomicBool,
    started_event: std::sync::atomic::AtomicBool,
    finished_event: std::sync::atomic::AtomicBool,
    live_actor: std::sync::atomic::AtomicBool,
    observations: Mutex<Vec<GraphObservation>>,
}

static SESSIONS: LazyLock<Mutex<HashMap<RunKey, Arc<RunSession>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

impl RunSession {
    pub(super) fn new(
        bindings: Arc<RunBindings>,
        config: Arc<Config>,
        flow: Flow,
        trigger: FlowRunTrigger,
        cursor: u64,
    ) -> Arc<Self> {
        let live_actor = trigger != FlowRunTrigger::Resume;
        Arc::new(Self {
            run_id: bindings.run_id.clone(),
            flow_id: bindings.flow_id.clone(),
            bindings: std::sync::RwLock::new(bindings),
            config,
            flow: tokio::sync::RwLock::new(flow),
            trigger: Mutex::new(trigger),
            cursor: std::sync::atomic::AtomicU64::new(cursor),
            poll_lock: tokio::sync::Mutex::new(()),
            watchers: Mutex::new(Vec::new()),
            poller_started: std::sync::atomic::AtomicBool::new(false),
            started_event: std::sync::atomic::AtomicBool::new(false),
            finished_event: std::sync::atomic::AtomicBool::new(false),
            live_actor: std::sync::atomic::AtomicBool::new(live_actor),
            observations: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn subscribe(&self) -> tokio::sync::mpsc::UnboundedReceiver<RunEvent> {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        self.watchers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(sender);
        receiver
    }

    pub(super) fn publish(&self, event: RunEvent) {
        self.watchers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|watcher| watcher.send(event.clone()).is_ok());
    }

    pub(super) fn add_observation(&self, observation: GraphObservation) {
        self.observations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(observation);
    }

    pub(super) fn take_observations(&self) -> Vec<GraphObservation> {
        std::mem::take(
            &mut *self
                .observations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    pub(super) fn cursor(&self) -> u64 {
        self.cursor.load(std::sync::atomic::Ordering::Acquire)
    }

    pub(super) fn set_trigger(&self, trigger: FlowRunTrigger) {
        *self
            .trigger
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = trigger;
    }

    pub(super) fn trigger(&self) -> FlowRunTrigger {
        *self
            .trigger
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn replace_bindings(&self, bindings: Arc<RunBindings>) {
        *self
            .bindings
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = bindings;
    }

    pub(super) fn current_bindings(&self) -> Arc<RunBindings> {
        Arc::clone(
            &self
                .bindings
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    pub(super) async fn replace_flow(&self, flow: Flow) {
        *self.flow.write().await = flow;
    }

    pub(super) async fn flow(&self) -> Flow {
        self.flow.read().await.clone()
    }

    pub(super) fn flow_id(&self) -> &str {
        &self.flow_id
    }

    pub(super) fn mark_started(&self) -> bool {
        self.live_actor
            .store(true, std::sync::atomic::Ordering::Release);
        !self
            .started_event
            .swap(true, std::sync::atomic::Ordering::AcqRel)
    }

    pub(super) fn mark_finished(&self) -> bool {
        self.live_actor
            .store(false, std::sync::atomic::Ordering::Release);
        !self
            .finished_event
            .swap(true, std::sync::atomic::Ordering::AcqRel)
    }

    pub(super) fn mark_parked(&self) {
        self.live_actor
            .store(false, std::sync::atomic::Ordering::Release);
    }

    pub(super) fn mark_resumed(&self) {
        self.live_actor
            .store(true, std::sync::atomic::Ordering::Release);
    }

    pub(super) fn is_live_actor(&self) -> bool {
        self.live_actor.load(std::sync::atomic::Ordering::Acquire)
    }

    pub(super) fn key(&self) -> RunKey {
        let bindings = self
            .bindings
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        RunKey::new(&bindings.storage(), &self.run_id)
    }

    /// Start the one PollRun task for this process/run owner.
    pub(super) fn start_polling(self: &Arc<Self>, proxy: tinybus::Proxy) -> Result<(), String> {
        if self
            .poller_started
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return Ok(());
        }
        let session = Arc::clone(self);
        spawn_poller(async move {
            loop {
                match session.drain(&proxy, true).await {
                    Ok(true) => {
                        remove(&session);
                        return;
                    }
                    Ok(false) => tokio::time::sleep(Duration::from_millis(50)).await,
                    Err(error) => {
                        tracing::warn!(
                            target: "flows",
                            run_id = %session.run_id,
                            error = %error,
                            "[flows] PollRun failed; retrying the owner-scoped cursor"
                        );
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                }
            }
        })
        .map(|_| ())
        .inspect_err(|_| {
            self.poller_started
                .store(false, std::sync::atomic::Ordering::Release);
        })
    }

    /// Poll all currently available pages. The lock prevents a resume waiter
    /// and the background projector from reading and publishing one sequence
    /// twice. `project=false` is used only to prime a process-restarted parked
    /// run before admitting its resume.
    pub(super) async fn drain(
        &self,
        proxy: &tinybus::Proxy,
        project: bool,
    ) -> Result<bool, String> {
        let _poll = self.poll_lock.lock().await;
        loop {
            let cursor = self.cursor();
            let bindings = Arc::clone(
                &self
                    .bindings
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            let request = runs::RunControlRequest {
                storage: bindings.storage(),
                run_id: self.run_id.clone(),
                cursor,
            };
            let page: runs::PollRun = proxy
                .call(runs::methods::POLL_RUN, (request,))
                .await
                .map_err(|error| error.wire_message())?;
            for event in page.events {
                self.consume(&event, project).await;
            }
            if page.done {
                return Ok(true);
            }
            if page.next_cursor <= cursor {
                return Ok(false);
            }
            self.cursor
                .store(page.next_cursor, std::sync::atomic::Ordering::Release);
        }
    }

    /// Advance the owner-scoped cursor for one unseen event, then project and
    /// fan it out exactly once. Private module event kinds still advance the
    /// cursor even when OpenHuman has no public event for them.
    pub(super) async fn consume(&self, event: &RunEvent, project: bool) -> bool {
        if event.seq <= self.cursor() {
            return false;
        }
        self.cursor
            .store(event.seq, std::sync::atomic::Ordering::Release);
        if project {
            projection::project(self, event).await;
            self.publish(event.clone());
        }
        true
    }
}

pub(crate) fn insert(session: Arc<RunSession>) -> Result<(), String> {
    let bindings = session
        .bindings
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let key = RunKey::new(&bindings.storage(), &session.run_id);
    drop(bindings);
    let mut sessions = SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if sessions.contains_key(&key) {
        return Err(format!(
            "flow run '{}' already has a host session",
            session.run_id
        ));
    }
    sessions.insert(key, session);
    Ok(())
}

pub(crate) fn get(
    storage: &tinyflows_bus::catalog::CatalogStorage,
    run_id: &str,
) -> Option<Arc<RunSession>> {
    SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&RunKey::new(storage, run_id))
        .cloned()
}

pub(crate) fn remove(session: &Arc<RunSession>) {
    let bindings = session
        .bindings
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let key = RunKey::new(&bindings.storage(), &session.run_id);
    let mut sessions = SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if sessions
        .get(&key)
        .is_some_and(|current| Arc::ptr_eq(current, session))
    {
        sessions.remove(&key);
    }
}

// A reply can be followed immediately by a synchronous catalog call. Keep
// PollRun and its connection-lock waiters off that caller's runtime, and keep
// projecting events if the invocation's runtime is no longer driven.
fn spawn_poller(
    work: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<tokio::task::JoinHandle<()>, String> {
    crate::modules::runtime::spawn_cleanup(work).map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
