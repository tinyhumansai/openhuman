//! High-level orchestration control plane over TinyAgents' detached-task registry.
//!
//! This session used to own a second, bespoke process-local task table
//! (`HashMap<String, AgentRecord>` + `HashMap<String, JoinHandle>` plus a
//! `Notify` plus a hand-rolled terminal sweep). TinyAgents'
//! [`DetachedTaskRegistry`] is a strict superset of that: it owns the status
//! watch channels, cooperative cancellation tokens, hard-abort handles,
//! owner-scoped lookup, the wait/timeout loop, and the soft-cap terminal sweep.
//! `spawn_agent` / `wait_agents` / `abort_all` are now thin product wrappers
//! over `register` / `wait` / `cancel_all`, following the shape
//! [`running_subagents`](super::running_subagents) already established.
//!
//! What stays host-side is exactly the product layer: the
//! [`DomainEvent::AgentOrchestrationSpawned`] /
//! [`DomainEvent::AgentOrchestrationCompleted`] /
//! [`DomainEvent::AgentOrchestrationFailed`] event-bus bridge, the parent
//! [`AgentProgress`] fan-out, agent-definition resolution, and the
//! [`AgentSnapshot`] response shape callers already consume.
//!
//! One behavioural consequence of the crate registry is worth knowing: `wait`
//! **prunes** an entry once it observes a terminal status, so a child can be
//! waited on once. Every caller in-tree spawns and waits exactly once, and this
//! is the same contract `running_subagents` runs under.

use super::types::{
    AgentSnapshot, SpawnAgentRequest, SpawnAgentResponse, WaitAgentOptions, WaitAgentResponse,
};
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::openhuman::agent::harness::definition::{AgentDefinition, AgentDefinitionRegistry};
use crate::openhuman::agent::harness::fork_context::{
    current_parent, with_parent_context, ParentExecutionContext,
};
use crate::openhuman::agent::harness::subagent_runner::{
    run_subagent, SubagentRunOptions, SubagentRunOutcome,
};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent::tinyagents::orchestration::{
    shared_steering_registry, DetachedTaskRegistry, DetachedTaskRegistryError,
    DetachedTaskWaitOutcome, OrchestrationTaskStatus, TaskId,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use thiserror::Error;
use tinyagents_harness::CancellationToken;
use tokio::sync::{mpsc, watch};
use tokio::time::{Duration, Instant};

#[derive(Debug, Error)]
pub enum OrchestrationError {
    #[error("agent orchestration requires an active parent agent turn")]
    NoParentContext,
    #[error("agent definition registry has not been initialized")]
    RegistryUnavailable,
    #[error("agent definition '{0}' not found")]
    DefinitionNotFound(String),
    #[error("orchestration agent '{0}' not found")]
    AgentNotFound(String),
    #[error("agent_id and prompt are required")]
    InvalidSpawnRequest,
    #[error("orchestration task registry unavailable: {0:?}")]
    Registry(DetachedTaskRegistryError),
}

/// Soft cap on live children per session. Terminal entries are only swept when
/// the table grows past this, matching [`super::running_subagents`].
const REGISTRY_SOFT_CAP: usize = 256;

/// Chunk length for an unbounded `wait_agents`. The crate's `wait` takes a
/// concrete deadline, so a `timeout_ms: None` request loops over these until a
/// terminal status lands — which preserves the old "wait forever" contract
/// without inventing a fake far-future instant.
const UNBOUNDED_WAIT_CHUNK: Duration = Duration::from_secs(3600);

/// Product metadata retained alongside each detached child's executor handles.
///
/// `status_tx` rides in the metadata deliberately: it is what lets
/// [`AgentOrchestrationSession::abort_all`] publish a terminal `Cancelled`
/// status to an in-flight `wait_agents` before the crate hard-aborts the task.
/// Without it, cancelling drops the sender and a concurrent waiter sees a closed
/// channel instead of a cancellation.
#[derive(Clone)]
struct ChildMetadata {
    agent_id: String,
    parent_agent_id: Option<String>,
    prompt: String,
    created_at: String,
    metadata: BTreeMap<String, String>,
    status_tx: Arc<watch::Sender<ChildState>>,
}

/// Live status plus the terminal payload a snapshot reports.
#[derive(Clone)]
struct ChildState {
    status: OrchestrationTaskStatus,
    result_summary: Option<String>,
    error: Option<String>,
    updated_at: String,
}

impl ChildState {
    fn pending(updated_at: String) -> Self {
        Self {
            status: OrchestrationTaskStatus::Pending,
            result_summary: None,
            error: None,
            updated_at,
        }
    }

    fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }
}

type ChildRegistry = DetachedTaskRegistry<ChildMetadata, ChildState>;

#[derive(Clone)]
pub struct AgentOrchestrationSession {
    session_id: String,
    registry: Arc<ChildRegistry>,
}

impl AgentOrchestrationSession {
    /// Create an orchestration session backed by its own TinyAgents
    /// detached-task registry.
    ///
    /// The `session_id` identifies the parent orchestration run in emitted
    /// [`DomainEvent`] payloads and is the registry's owner id, so one session
    /// can never wait on or cancel another's children.
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            registry: Arc::new(DetachedTaskRegistry::new(
                shared_steering_registry().clone(),
                REGISTRY_SOFT_CAP,
                ChildState::is_terminal,
            )),
        }
    }

    /// Return the stable parent orchestration session id.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Spawn a child agent from the active parent agent turn.
    ///
    /// `request` must provide a non-empty `agent_id` and `prompt`; optional
    /// context, toolkit, model, parent id, and metadata are carried into the
    /// child record and sub-agent run options. On success this returns the
    /// accepted child id and initial status while a background task executes the
    /// child through [`run_subagent`].
    ///
    /// Returns [`OrchestrationError::NoParentContext`] when called outside an
    /// agent turn, [`OrchestrationError::InvalidSpawnRequest`] for an empty
    /// agent id or prompt, [`OrchestrationError::RegistryUnavailable`] when the
    /// agent definition registry is not initialized, or
    /// [`OrchestrationError::DefinitionNotFound`] for an unknown agent id. Side
    /// effects include registering the child in the detached-task registry,
    /// publishing a [`DomainEvent::AgentOrchestrationSpawned`] event, and
    /// emitting parent progress when available.
    pub async fn spawn_agent(
        &self,
        request: SpawnAgentRequest,
    ) -> Result<SpawnAgentResponse, OrchestrationError> {
        let parent = current_parent().ok_or(OrchestrationError::NoParentContext)?;
        let definition = resolve_definition(&request)?;
        self.spawn_agent_with_definition(parent, definition, request)
            .await
    }

    /// Wait for one or more child agents to reach terminal status.
    ///
    /// `options.orchestration_ids` names the children to observe. An empty id
    /// list returns the current full session snapshot immediately. When
    /// `timeout_ms` is present, the wait returns a partial response with
    /// `completed = false` after the timeout instead of failing; the deadline
    /// covers the whole id list, not each child in turn.
    ///
    /// Returns [`OrchestrationError::AgentNotFound`] if any requested child id
    /// is unknown to this session's registry.
    pub async fn wait_agents(
        &self,
        options: WaitAgentOptions,
    ) -> Result<WaitAgentResponse, OrchestrationError> {
        if options.orchestration_ids.is_empty() {
            return Ok(WaitAgentResponse {
                completed: true,
                agents: self.all_snapshots()?,
            });
        }

        let deadline = options
            .timeout_ms
            .map(|ms| Instant::now() + Duration::from_millis(ms));
        let mut agents = Vec::with_capacity(options.orchestration_ids.len());
        let mut completed = true;

        for id in &options.orchestration_ids {
            let task_id = TaskId::new(id.clone());
            // Metadata is only retained while the entry is live, so capture it
            // before `wait` prunes a terminal child.
            let metadata = self
                .registry
                .snapshot(&task_id, &self.session_id)
                .map_err(|err| self.lookup_error(id, err))?
                .metadata;

            let state = self.wait_one(&task_id, deadline).await?;
            if !state.is_terminal() {
                completed = false;
            }
            agents.push(snapshot_of(id, &metadata, &state));
        }

        Ok(WaitAgentResponse { completed, agents })
    }

    /// Abort every in-flight child and mark non-terminal children
    /// [`OrchestrationTaskStatus::Cancelled`].
    ///
    /// Used by the workflow engine on stop/interrupt to drain a session's
    /// running children. Idempotent — the crate registry cancels the
    /// cooperative token and the hard abort handle, and the `Cancelled` status
    /// is published on each child's watch channel first so a concurrent
    /// `wait_agents` resolves with a cancellation rather than a closed channel.
    pub async fn abort_all(&self) {
        let cancelled = match self.registry.cancel_all() {
            Ok(cancelled) => cancelled,
            Err(err) => {
                log::warn!("[agent_orchestration] abort_all could not drain registry: {err:?}");
                return;
            }
        };
        for entry in &cancelled {
            if entry.status.is_terminal() {
                continue;
            }
            let _ = entry.metadata.status_tx.send(ChildState {
                status: OrchestrationTaskStatus::Cancelled,
                result_summary: entry.status.result_summary.clone(),
                error: entry.status.error.clone(),
                updated_at: now(),
            });
        }
        log::debug!(
            "[agent_orchestration] abort_all session={} cancelled={}",
            self.session_id,
            cancelled.len()
        );
    }

    /// Wait one child to a terminal status, honouring the shared `deadline`.
    async fn wait_one(
        &self,
        task_id: &TaskId,
        deadline: Option<Instant>,
    ) -> Result<ChildState, OrchestrationError> {
        loop {
            let slice = match deadline {
                Some(deadline) => deadline.saturating_duration_since(Instant::now()),
                None => UNBOUNDED_WAIT_CHUNK,
            };
            let outcome = self
                .registry
                .wait(task_id, &self.session_id, slice)
                .await
                .map_err(|err| self.lookup_error(task_id.as_str(), err))?;
            match outcome {
                DetachedTaskWaitOutcome::Terminal(state) => return Ok(state),
                DetachedTaskWaitOutcome::TimedOut(state) => {
                    if deadline.is_some() {
                        return Ok(state);
                    }
                    // Unbounded wait: the chunk expired, keep waiting.
                }
            }
        }
    }

    /// Every child snapshot still live in this session's registry, ordered by
    /// creation time.
    fn all_snapshots(&self) -> Result<Vec<AgentSnapshot>, OrchestrationError> {
        let mut agents: Vec<AgentSnapshot> = self
            .registry
            .snapshots(Some(&self.session_id))
            .map_err(OrchestrationError::Registry)?
            .into_iter()
            .map(|entry| snapshot_of(entry.task_id.as_str(), &entry.metadata, &entry.status))
            .collect();
        agents.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(agents)
    }

    fn lookup_error(&self, id: &str, err: DetachedTaskRegistryError) -> OrchestrationError {
        match err {
            DetachedTaskRegistryError::Unknown | DetachedTaskRegistryError::NotOwned => {
                OrchestrationError::AgentNotFound(id.to_string())
            }
            other => OrchestrationError::Registry(other),
        }
    }

    async fn spawn_agent_with_definition(
        &self,
        parent: ParentExecutionContext,
        definition: AgentDefinition,
        request: SpawnAgentRequest,
    ) -> Result<SpawnAgentResponse, OrchestrationError> {
        let agent_id = request.agent_id.trim().to_string();
        let prompt = request.prompt.trim().to_string();
        if agent_id.is_empty() || prompt.is_empty() {
            return Err(OrchestrationError::InvalidSpawnRequest);
        }

        let orchestration_id = format!("agent-{}", uuid::Uuid::new_v4());
        let created_at = now();
        let (status_tx, status_rx) = watch::channel(ChildState::pending(created_at.clone()));
        let status_tx = Arc::new(status_tx);
        let metadata = ChildMetadata {
            agent_id: agent_id.clone(),
            parent_agent_id: request.parent_agent_id.clone(),
            prompt: prompt.clone(),
            created_at,
            metadata: request.metadata.clone(),
            status_tx: status_tx.clone(),
        };

        BUS.publish(DomainEvent::AgentOrchestrationSpawned {
            session_id: self.session_id.clone(),
            orchestration_id: orchestration_id.clone(),
            agent_id: agent_id.clone(),
            parent_agent_id: request.parent_agent_id,
        });

        let progress_sink = parent.on_progress.clone();
        if let Some(progress) = progress_sink.clone() {
            let resolved_display_name = AgentDefinitionRegistry::global()
                .and_then(|reg| reg.get(&agent_id))
                .map(|def| def.display_name().to_string());
            let _ = progress
                .send(AgentProgress::SubagentSpawned {
                    agent_id: agent_id.clone(),
                    task_id: orchestration_id.clone(),
                    mode: "typed".to_string(),
                    dedicated_thread: false,
                    prompt_chars: prompt.chars().count(),
                    prompt: prompt.clone(),
                    worker_thread_id: None,
                    display_name: resolved_display_name,
                })
                .await;
        }

        let parent_workspace_descriptor = parent.workspace_descriptor.clone();
        let parent_worktree_action_dir = parent_workspace_descriptor
            .as_ref()
            .map(|descriptor| descriptor.root.clone());
        if let Some(descriptor) = parent_workspace_descriptor.as_ref() {
            tracing::debug!(
                orchestration_id = %orchestration_id,
                agent_id = %agent_id,
                workspace_root = %descriptor.root.display(),
                policy_id = %descriptor.policy_id,
                "[agent_orchestration] inheriting parent workspace descriptor"
            );
        }

        let options = SubagentRunOptions {
            skill_filter_override: None,
            toolkit_override: request.toolkit,
            context: request.context,
            model_override: request.model,
            task_id: Some(orchestration_id.clone()),
            worker_thread_id: None,
            initial_history: None,
            checkpoint_dir: None,
            worktree_action_dir: parent_worktree_action_dir,
            workspace_descriptor: parent_workspace_descriptor,
            // Not a dropped value: this is the harness's mid-run steering
            // channel, and this path has never offered live steering — the
            // orchestration control plane's `message_agent` recorded metadata
            // only, and it is gone. The child's *registry* entry does carry a
            // cooperative `CancellationToken` and an `AbortHandle` below, which
            // is what `abort_all` uses.
            run_queue: None,
        };

        let session = self.clone();
        let task_id = orchestration_id.clone();
        let task_status_tx = status_tx.clone();
        let task_agent_id = agent_id.clone();
        // Captured on *this* task: a `tokio::task_local` does not cross
        // `tokio::spawn`, so the turn's origin label and workspace root are
        // carried across the same boundary the parent execution context
        // already is. Without the origin the spawned agent's external-effect
        // tools reach the approval gate unlabelled and are refused.
        let handle = tokio::spawn(crate::openhuman::agent::turn_origin::propagate(
            crate::openhuman::agent::turn_workspace::propagate(async move {
                mark_running(&task_status_tx);
                let result = with_parent_context(parent, async move {
                    run_subagent(&definition, &prompt, options).await
                })
                .await;
                session
                    .finish_agent(
                        &task_id,
                        &task_agent_id,
                        &task_status_tx,
                        progress_sink,
                        result,
                    )
                    .await;
            }),
        ));

        self.registry
            .register(
                TaskId::new(orchestration_id.clone()),
                self.session_id.clone(),
                metadata,
                status_rx,
                CancellationToken::new(),
                handle.abort_handle(),
            )
            .map_err(|err| {
                log::error!("[agent_orchestration] duplicate detached child id: {err}");
                // Registration failed, so the registry holds no record of this
                // task and `abort_all`/`cancel_all` can never reach it. Dropping
                // `handle` here would only detach the `JoinHandle` — the spawned
                // task keeps running orphaned. Abort it explicitly so a failed
                // spawn never leaves a live, unreachable child behind.
                handle.abort();
                OrchestrationError::InvalidSpawnRequest
            })?;

        log::debug!(
            "[agent_orchestration] spawned session={} orchestration_id={} agent_id={}",
            self.session_id,
            orchestration_id,
            agent_id
        );

        Ok(SpawnAgentResponse {
            orchestration_id,
            agent_id,
            status: OrchestrationTaskStatus::Pending,
        })
    }

    async fn finish_agent(
        &self,
        orchestration_id: &str,
        agent_id: &str,
        status_tx: &watch::Sender<ChildState>,
        progress_sink: Option<mpsc::Sender<AgentProgress>>,
        result: Result<SubagentRunOutcome, crate::openhuman::agent::harness::SubagentRunError>,
    ) {
        // A cancelled child has already reached a terminal status via
        // `abort_all`; do not overwrite it with a late completion.
        if status_tx.borrow().is_terminal() {
            return;
        }

        match result {
            Ok(outcome) => {
                let _ = status_tx.send(ChildState {
                    status: OrchestrationTaskStatus::Completed,
                    result_summary: Some(outcome.output.clone()),
                    error: None,
                    updated_at: now(),
                });
                BUS.publish(DomainEvent::AgentOrchestrationCompleted {
                    session_id: self.session_id.clone(),
                    orchestration_id: orchestration_id.to_string(),
                    agent_id: outcome.agent_id.clone(),
                    elapsed_ms: outcome.elapsed.as_millis() as u64,
                    output_chars: outcome.output.chars().count(),
                    iterations: outcome.iterations,
                });
                if let Some(progress) = progress_sink {
                    let _ = progress
                        .send(AgentProgress::SubagentCompleted {
                            agent_id: outcome.agent_id.clone(),
                            task_id: orchestration_id.to_string(),
                            elapsed_ms: outcome.elapsed.as_millis() as u64,
                            iterations: outcome.iterations as u32,
                            output_chars: outcome.output.chars().count(),
                            output: outcome.output.clone(),
                            // Not a dropped value: these three describe a
                            // worker's *own* isolated checkout, and this path
                            // never creates one — it only inherits the parent's
                            // descriptor (above). `spawn_parallel_agents`
                            // populates them from the descriptor it freshly
                            // created per worker, and reports `None` for an
                            // inherited one for the same reason.
                            worktree_path: None,
                            changed_files: Vec::new(),
                            dirty_status: None,
                        })
                        .await;
                }
            }
            Err(error) => {
                let message = error.to_string();
                let _ = status_tx.send(ChildState {
                    status: OrchestrationTaskStatus::Failed,
                    result_summary: None,
                    error: Some(message.clone()),
                    updated_at: now(),
                });
                BUS.publish(DomainEvent::AgentOrchestrationFailed {
                    session_id: self.session_id.clone(),
                    orchestration_id: orchestration_id.to_string(),
                    agent_id: agent_id.to_string(),
                    error: message.clone(),
                });
                if let Some(progress) = progress_sink {
                    let _ = progress
                        .send(AgentProgress::SubagentFailed {
                            agent_id: agent_id.to_string(),
                            task_id: orchestration_id.to_string(),
                            error: message,
                        })
                        .await;
                }
            }
        }
    }
}

/// Publish `Running` unless the child already reached a terminal status.
fn mark_running(status_tx: &watch::Sender<ChildState>) {
    let updated_at = now();
    status_tx.send_if_modified(|state| {
        if state.is_terminal() || state.status == OrchestrationTaskStatus::Running {
            return false;
        }
        state.status = OrchestrationTaskStatus::Running;
        state.updated_at = updated_at;
        true
    });
}

fn snapshot_of(
    orchestration_id: &str,
    metadata: &ChildMetadata,
    state: &ChildState,
) -> AgentSnapshot {
    AgentSnapshot {
        orchestration_id: orchestration_id.to_string(),
        agent_id: metadata.agent_id.clone(),
        parent_agent_id: metadata.parent_agent_id.clone(),
        status: state.status,
        prompt: metadata.prompt.clone(),
        result_summary: state.result_summary.clone(),
        error: state.error.clone(),
        created_at: metadata.created_at.clone(),
        updated_at: state.updated_at.clone(),
        metadata: metadata.metadata.clone(),
    }
}

fn resolve_definition(request: &SpawnAgentRequest) -> Result<AgentDefinition, OrchestrationError> {
    let agent_id = request.agent_id.trim();
    if agent_id.is_empty() || request.prompt.trim().is_empty() {
        return Err(OrchestrationError::InvalidSpawnRequest);
    }
    let registry =
        AgentDefinitionRegistry::global().ok_or(OrchestrationError::RegistryUnavailable)?;
    registry
        .get(agent_id)
        .cloned()
        .ok_or_else(|| OrchestrationError::DefinitionNotFound(agent_id.to_string()))
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
