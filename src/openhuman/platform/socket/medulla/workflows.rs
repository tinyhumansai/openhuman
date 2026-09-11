//! The workflow plane of the medulla harness protocol: advertising this host's
//! saved workflow graphs, and serving the reads (and the one authoring turn)
//! the orchestrator round-trips back to whoever owns them.
//!
//! **Why a trait and not a direct call into `flows`.** Two different hosts embed
//! this transport — openhuman over its SQLite `flows` store, and medulla-public
//! over its own layered JSON workflow store — and they share nothing but the
//! wire shape. So the store side is a host-supplied [`WorkflowBridge`] the host
//! installs once at startup ([`set_workflow_bridge`]), and the engine types stay
//! entirely on the host's side of the seam: a graph, a node-kind catalog and a
//! run list all cross as opaque [`serde_json::Value`], exactly as they do in
//! medulla-v1's `WorkflowCatalog` port. Nothing in this module parses a node, an
//! edge, or a config field.
//!
//! **Why every path answers.** The server correlates a request by `requestId`
//! and waits on a deadline — ten seconds for a read, ten *minutes* for a copilot
//! turn. A dropped request is therefore not a cheap no-op: it burns that whole
//! window and, for `copilot`, holds a promise open for the rest of the cycle. So
//! a missing bridge, an unknown op argument, a bridge error and even a panicking
//! bridge method all become a `medulla:workflow_result` with `ok: false` and a
//! readable message. The only way to stay silent is for the socket to be gone,
//! which the server already handles by retiring the waiter on disconnect.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use futures::FutureExt;
use parking_lot::RwLock;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::payloads::{
    CopilotOutcome, RegisterWorkflows, WorkflowDescriptor, WorkflowOp, WorkflowRequest,
    WorkflowResult, EVENT_REGISTER_WORKFLOWS, EVENT_WORKFLOW_RESULT,
};

/// The store side of the workflow plane, supplied by the embedding host.
///
/// The reads are synchronous because every host backs them with a local store;
/// they are dispatched on a blocking thread so a slow store cannot stall the
/// socket runtime. `copilot` is `async` because it is not a read at all — it is
/// a whole agent turn on the host's own authoring copilot.
///
/// Every method returns `Result<_, String>` rather than a host error type: the
/// error is going straight onto a model's prompt surface as a tool result, so it
/// must already be a sentence a reader can act on.
#[async_trait]
pub trait WorkflowBridge: Send + Sync {
    /// The adverts this host publishes. Cheap and side-effect free — it is
    /// called on every (re)connect and on every store change.
    fn list(&self) -> Vec<WorkflowDescriptor>;

    /// Fallible advert read used by registration.
    ///
    /// The default preserves compatibility for hosts whose list cannot fail.
    /// Store-backed hosts override it so a transient read failure does not look
    /// like a successful empty replacement batch.
    fn try_list(&self) -> Result<Vec<WorkflowDescriptor>, String> {
        Ok(self.list())
    }

    /// One workflow's detail, graph included, as the host's own JSON shape.
    fn get(&self, id: &str) -> Result<Value, String>;

    /// The node-kind catalog, optionally narrowed to a single `kind`. This is an
    /// authoring aid: a manager reads it to brief the copilot accurately.
    fn node_kinds(&self, kind: Option<&str>) -> Result<Value, String>;

    /// Recent runs of one workflow, for visibility into a delegated graph.
    fn runs(&self, id: &str) -> Result<Value, String>;

    /// Run one authoring turn on the host's copilot. `workflow_id` absent means
    /// "create a new workflow". The outcome must be derived from a re-read of
    /// the host's own store rather than from the model's claim about what it
    /// did — that is the accountability property the whole delegation rests on.
    async fn copilot(
        &self,
        instruction: &str,
        workflow_id: Option<&str>,
    ) -> Result<CopilotOutcome, String>;

    /// Which roster agent owns these workflows, when the host runs more than
    /// one identity. Stamped onto the advert batch so the backend can route a
    /// delegated workflow to its owner. Defaults to unset — the backend then
    /// treats the whole socket as the owner.
    fn agent_id(&self) -> Option<String> {
        None
    }

    /// The action directory associated with this bridge's authenticated
    /// identity, when the host has one. Capability probes use this instead of
    /// reloading process-global active-user state.
    fn action_dir(&self) -> Option<String> {
        None
    }
}

#[derive(Clone)]
pub(super) struct BridgeGeneration {
    pub(super) bridge: Option<Arc<dyn WorkflowBridge>>,
    pub(super) cancel: CancellationToken,
}

struct BridgeState {
    bridge: Option<Arc<dyn WorkflowBridge>>,
    generation: CancellationToken,
}

struct ConnectionGeneration {
    current: CancellationToken,
}

impl ConnectionGeneration {
    fn disconnected() -> Self {
        let current = CancellationToken::new();
        current.cancel();
        Self { current }
    }

    fn snapshot(&self) -> CancellationToken {
        self.current.clone()
    }

    fn begin(&mut self) {
        self.current.cancel();
        self.current = CancellationToken::new();
    }

    fn end(&mut self) {
        self.current.cancel();
        *self = Self::disconnected();
    }
}

static BRIDGE_STATE: OnceLock<RwLock<BridgeState>> = OnceLock::new();
static CONNECTION_GENERATION: OnceLock<RwLock<ConnectionGeneration>> = OnceLock::new();

fn bridge_state() -> &'static RwLock<BridgeState> {
    BRIDGE_STATE.get_or_init(|| {
        RwLock::new(BridgeState {
            bridge: None,
            generation: CancellationToken::new(),
        })
    })
}

fn connection_generation_state() -> &'static RwLock<ConnectionGeneration> {
    CONNECTION_GENERATION.get_or_init(|| RwLock::new(ConnectionGeneration::disconnected()))
}

pub(super) fn bridge_generation() -> BridgeGeneration {
    let state = bridge_state().read();
    BridgeGeneration {
        bridge: state.bridge.clone(),
        cancel: state.generation.clone(),
    }
}

/// A snapshot of the token for the currently-authenticated socket lifetime.
///
/// Visible to the whole `socket` module, not just `medulla`: `event_handlers`
/// spawns hosted-brain work that must not emit its result onto a *later*
/// connection, and it is a sibling of this module rather than a child.
pub(in crate::openhuman::platform::socket) fn connection_generation() -> CancellationToken {
    connection_generation_state().read().snapshot()
}

/// Run `deliver` only while `token`'s generation is still live, holding the
/// generation lock across the decision.
///
/// A plain `token.is_cancelled()` followed by a send is a race: the socket loop
/// can end the generation and drain the emit channel in the gap, after which the
/// handler's send lands in an empty channel and is flushed onto the *next*
/// connection — exactly what the check was meant to prevent.
///
/// Taking the read guard closes it, because [`end_connection_generation`] needs
/// the write guard to cancel. Either this runs first, and the message is queued
/// before the drain that will discard it; or the end runs first, and `token` is
/// already cancelled here. `deliver` must therefore stay cheap and must not
/// block, await, or re-enter this module — the one caller does a JSON encode and
/// an unbounded send.
///
/// Returns `None` when the generation has ended.
pub(in crate::openhuman::platform::socket) fn with_live_connection<R>(
    token: &CancellationToken,
    deliver: impl FnOnce() -> R,
) -> Option<R> {
    let _generation = connection_generation_state().read();
    if token.is_cancelled() {
        return None;
    }
    Some(deliver())
}

/// Start a new authenticated-socket lifetime.
///
/// Called by the `ready` handler before it advertises workflows. Work received
/// before `ready`, while reconnecting, or after a disconnect sees a cancelled
/// generation and is discarded instead of outliving the backend waiter that
/// sent it.
pub(in crate::openhuman::platform::socket) fn begin_connection_generation() {
    connection_generation_state().write().begin();
}

/// End the current authenticated-socket lifetime and cancel its work.
pub(in crate::openhuman::platform::socket) fn end_connection_generation() {
    connection_generation_state().write().end();
}

fn replace_bridge(bridge: Option<Arc<dyn WorkflowBridge>>) -> CancellationToken {
    let mut state = bridge_state().write();
    state.generation.cancel();
    let generation = CancellationToken::new();
    state.bridge = bridge;
    state.generation = generation.clone();
    generation
}

/// Orders overlapping snapshot emissions by successful registration revision.
///
/// Reads stay concurrent and off the socket runtime. A newer successful read
/// supersedes every older one, while a newer failed read does not suppress an
/// older successful snapshot. The mutex makes the revision check plus the
/// awaited socket enqueue one ordered operation.
struct RegistrationSequencer {
    next_revision: AtomicU64,
    last_emitted: tokio::sync::Mutex<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SequencedEmit {
    Emitted,
    Superseded,
    Failed,
}

impl RegistrationSequencer {
    const fn new() -> Self {
        Self {
            next_revision: AtomicU64::new(0),
            last_emitted: tokio::sync::Mutex::const_new(0),
        }
    }

    fn begin(&self) -> u64 {
        self.next_revision.fetch_add(1, Ordering::Relaxed) + 1
    }

    async fn emit_if_newer<F, Fut>(&self, revision: u64, emit: F) -> SequencedEmit
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = bool>,
    {
        let mut last_emitted = self.last_emitted.lock().await;
        if revision <= *last_emitted {
            return SequencedEmit::Superseded;
        }
        if !emit().await {
            return SequencedEmit::Failed;
        }
        *last_emitted = revision;
        SequencedEmit::Emitted
    }
}

static REGISTRATION_SEQUENCE: RegistrationSequencer = RegistrationSequencer::new();

/// Install (or replace) the host's workflow bridge, and advertise what it holds.
///
/// Registering re-advertises immediately so a bridge installed after connect
/// still reaches a backend that has already seen this socket's `ready`.
pub fn set_workflow_bridge(bridge: Arc<dyn WorkflowBridge>) {
    replace_bridge(Some(bridge));
    emit_register_workflows();
}

/// Remove the installed bridge and tell the backend this socket now advertises
/// nothing. Sending an empty batch is the only way to retract: the backend
/// replaces a socket's whole entry on each registration and has no delete event.
pub fn clear_workflow_bridge() {
    let generation = replace_bridge(None);
    let connection_cancel = connection_generation();
    let revision = REGISTRATION_SEQUENCE.begin();
    tokio::spawn(async move {
        tokio::select! {
            biased;
            _ = generation.cancelled() => {}
            _ = connection_cancel.cancelled() => {}
            _ = async move {
                let batch = RegisterWorkflows {
                    workflows: Vec::new(),
                    agent_id: None,
                };
                REGISTRATION_SEQUENCE
                    .emit_if_newer(revision, || emit_batch(batch))
                    .await;
            } => {}
        }
    });
}

/// Emit `medulla:register_workflows` — the advert batch, sent on connect and
/// whenever the host signals its workflow set changed.
///
/// A host without a bridge stays silent rather than advertising an empty list:
/// "I have no workflow plane" and "my store is empty" are different claims, and
/// only the second one should clear a previously registered set.
pub fn emit_register_workflows() {
    let generation = bridge_generation();
    let Some(bridge) = generation.bridge else {
        log::debug!("[medulla] no workflow bridge installed — not advertising workflows");
        return;
    };
    let revision = REGISTRATION_SEQUENCE.begin();
    let cancel = generation.cancel;
    let connection_cancel = connection_generation();
    // `list()` reads the host's store, so keep it off the socket runtime; a
    // panicking bridge surfaces as a `JoinError` here instead of unwinding.
    tokio::spawn(async move {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                log::debug!("[medulla] discarded workflow registration from an old bridge");
            }
            _ = connection_cancel.cancelled() => {
                log::debug!("[medulla] discarded workflow registration from a closed socket");
            }
            _ = async move {
                match read_batch(bridge).await {
                    Ok(batch) => {
                        let count = batch.workflows.len();
                        match REGISTRATION_SEQUENCE
                            .emit_if_newer(revision, || emit_batch(batch))
                            .await
                        {
                            SequencedEmit::Emitted => {
                                log::info!("[medulla] advertised {count} workflows to backend");
                            }
                            SequencedEmit::Superseded => log::debug!(
                                "[medulla] workflow registration snapshot already superseded by a newer \
                                 successful read — not advertising"
                            ),
                            SequencedEmit::Failed => {
                                log::debug!("[medulla] workflow registration enqueue failed")
                            }
                        }
                    }
                    Err(err) => {
                        log::warn!("[medulla] workflow bridge list() failed — not advertising: {err}")
                    }
                }
            } => {}
        }
    });
}

/// The adverts this host currently publishes, for callers that need the list
/// itself rather than the registration frame — today the capability probe, which
/// carries the same descriptors in its reply bag. Empty when no bridge is
/// installed or the bridge failed, so a probe still answers.
pub async fn advertised_workflows() -> Vec<WorkflowDescriptor> {
    advertised_workflows_for(bridge_generation().bridge).await
}

pub(super) async fn advertised_workflows_for(
    bridge: Option<Arc<dyn WorkflowBridge>>,
) -> Vec<WorkflowDescriptor> {
    let Some(bridge) = bridge else {
        return Vec::new();
    };
    read_batch(bridge)
        .await
        .map(|batch| batch.workflows)
        .unwrap_or_default()
}

/// Read one advert batch off the bridge on a blocking thread. An error or panic
/// remains a failed read — never an empty replacement or a partial batch.
async fn read_batch(bridge: Arc<dyn WorkflowBridge>) -> Result<RegisterWorkflows, String> {
    match tokio::task::spawn_blocking(move || {
        Ok(RegisterWorkflows {
            workflows: bridge.try_list()?,
            agent_id: bridge.agent_id(),
        })
    })
    .await
    {
        Ok(batch) => batch,
        Err(err) => Err(format!("the workflow store failed to list: {err}")),
    }
}

async fn emit_batch(batch: RegisterWorkflows) -> bool {
    super::emit_awaited(EVENT_REGISTER_WORKFLOWS, batch).await
}

/// Handle an inbound `medulla:workflow_request`.
///
/// Returns immediately: the work runs on its own task so a `copilot` turn (up to
/// ten minutes of agent time) cannot hold the socket read loop, the same shape
/// `medulla:task_run` and `orch:tool_call` already use.
pub fn handle_workflow_request(request: WorkflowRequest) {
    let BridgeGeneration { bridge, cancel } = bridge_generation();
    let connection_cancel = connection_generation();
    tokio::spawn(async move {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                log::debug!("[medulla] discarded workflow request from an old bridge");
            }
            _ = connection_cancel.cancelled() => {
                log::debug!("[medulla] discarded workflow request from a closed socket");
            }
            _ = async move {
                let request_id = request.request_id.clone();
                let op = request.op;
                let outcome = match bridge {
                    Some(bridge) => dispatch(bridge, request).await,
                    None => Err("this agent has no workflow store installed".to_string()),
                };
                match &outcome {
                    Ok(_) => log::debug!("[medulla] workflow {op:?} request_id={request_id} ok"),
                    Err(err) => {
                        log::warn!("[medulla] workflow {op:?} request_id={request_id} failed: {err}")
                    }
                }
                super::emit_awaited(
                    EVENT_WORKFLOW_RESULT,
                    result_frame(request_id, outcome),
                )
                .await;
            } => {}
        }
    });
}

/// Answer a `medulla:workflow_request` this build could not decode.
///
/// A frame with an unknown `op` (a newer backend) or a missing field would
/// otherwise be dropped, and a dropped request costs the backend the op's whole
/// deadline. The `requestId` is recovered from the raw JSON — without one there
/// is nothing to correlate, so the frame really is unanswerable and is only
/// logged.
pub fn reject_unparsed_request(raw: &Value, reason: &str) {
    let Some(request_id) = raw.get("requestId").and_then(Value::as_str) else {
        log::warn!("[medulla] undecodable workflow_request carries no requestId — cannot answer");
        return;
    };
    super::emit(
        EVENT_WORKFLOW_RESULT,
        result_frame(
            request_id.to_string(),
            Err(format!("this agent could not read the request: {reason}")),
        ),
    );
}

/// Project a dispatch outcome onto the wire frame. Split out so the mapping
/// (which arm is populated, and that exactly one of them is) is directly
/// testable without a socket or a runtime.
fn result_frame(request_id: String, outcome: Result<Value, String>) -> WorkflowResult {
    match outcome {
        Ok(data) => WorkflowResult {
            request_id,
            ok: true,
            data: Some(data),
            error: None,
        },
        Err(error) => WorkflowResult {
            request_id,
            ok: false,
            data: None,
            error: Some(error),
        },
    }
}

/// Route one request to `bridge`. Every failure mode — a missing op argument, a
/// bridge error, a panicking bridge method — comes back as an `Err(String)` the
/// caller turns into `ok: false`.
///
/// The bridge is a parameter rather than read from the registry here so the
/// whole dispatch table is exercisable against a scripted bridge without
/// touching process-global state.
async fn dispatch(
    bridge: Arc<dyn WorkflowBridge>,
    request: WorkflowRequest,
) -> Result<Value, String> {
    match request.op {
        WorkflowOp::Get => {
            let id = require_workflow_id(&request, "get")?;
            blocking(move || bridge.get(&id)).await
        }
        WorkflowOp::NodeKinds => {
            let kind = request.kind.clone();
            blocking(move || bridge.node_kinds(kind.as_deref())).await
        }
        WorkflowOp::Runs => {
            let id = require_workflow_id(&request, "runs")?;
            blocking(move || bridge.runs(&id)).await
        }
        WorkflowOp::Copilot => {
            let instruction = request
                .instruction
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "workflow copilot requires a non-empty instruction".to_string())?;
            let turn = bridge.copilot(instruction, request.workflow_id.as_deref());
            let outcome = catching_panics(turn).await??;
            serde_json::to_value(outcome)
                .map_err(|err| format!("failed to serialize the copilot outcome: {err}"))
        }
    }
}

/// The `workflowId` an op needs, or a message naming what was missing. Blank is
/// treated as absent: an empty id could only ever miss in the host's store, and
/// saying so up front beats a "not found" the reader cannot interpret.
fn require_workflow_id(request: &WorkflowRequest, op: &str) -> Result<String, String> {
    request
        .workflow_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("workflow {op} requires a workflowId"))
}

/// Run a synchronous bridge read on a blocking thread, mapping a panic inside it
/// to an error rather than letting it unwind through the task.
async fn blocking<F>(read: F) -> Result<Value, String>
where
    F: FnOnce() -> Result<Value, String> + Send + 'static,
{
    match tokio::task::spawn_blocking(read).await {
        Ok(result) => result,
        Err(err) => Err(format!("the workflow store failed to answer: {err}")),
    }
}

/// The async twin of [`blocking`]: run a bridge turn that must be awaited on the
/// socket runtime — today only `copilot` — and map a panic inside it to an error.
///
/// `copilot` cannot go on a blocking thread (it is a whole agent turn, not a
/// store read), so it does not inherit `spawn_blocking`'s `JoinError` net. Left
/// bare, a panicking `copilot` would abort the task that owes the backend a
/// `workflow_result` and cost it the op's whole ten-minute deadline. Caught
/// in-place rather than on a supervised child task so cancelling the answering
/// task still cancels the turn.
async fn catching_panics<F, T>(turn: F) -> Result<T, String>
where
    F: Future<Output = T>,
{
    match AssertUnwindSafe(turn).catch_unwind().await {
        Ok(value) => Ok(value),
        Err(panic) => {
            let reason = panic
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown panic");
            log::error!("[medulla] workflow copilot panicked: {reason}");
            Err(format!(
                "the workflow store failed to answer: the copilot panicked: {reason}"
            ))
        }
    }
}

#[cfg(test)]
#[path = "workflows_tests.rs"]
mod tests;
