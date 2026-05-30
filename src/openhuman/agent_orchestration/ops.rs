//! In-memory high-level orchestration control plane.

use super::types::{
    AgentMessage, AgentOrchestrationEvent, AgentSnapshot, AgentStatus, CloseAgentRequest,
    FollowUpRequest, MessageAgentRequest, ResumeAgentRequest, SpawnAgentRequest,
    SpawnAgentResponse, WaitAgentOptions, WaitAgentResponse,
};
use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::harness::definition::{AgentDefinition, AgentDefinitionRegistry};
use crate::openhuman::agent::harness::fork_context::{
    current_parent, with_parent_context, ParentExecutionContext,
};
use crate::openhuman::agent::harness::subagent_runner::{
    run_subagent, SubagentRunOptions, SubagentRunOutcome,
};
use crate::openhuman::agent::progress::AgentProgress;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{mpsc, Mutex, Notify};
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

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
    #[error("orchestration agent '{0}' is already terminal")]
    AgentTerminal(String),
    #[error("agent_id and prompt are required")]
    InvalidSpawnRequest,
    #[error("message content is required")]
    InvalidMessage,
}

#[derive(Clone)]
pub struct AgentOrchestrationSession {
    session_id: String,
    state: Arc<Mutex<SessionState>>,
    notify: Arc<Notify>,
}

#[derive(Default)]
struct SessionState {
    agents: HashMap<String, AgentRecord>,
    tasks: HashMap<String, JoinHandle<()>>,
    events: Vec<AgentOrchestrationEvent>,
}

#[derive(Clone)]
struct AgentRecord {
    snapshot: AgentSnapshot,
    toolkit: Option<String>,
    model: Option<String>,
    progress_sink: Option<mpsc::Sender<AgentProgress>>,
}

impl AgentOrchestrationSession {
    /// Create an in-memory orchestration session.
    ///
    /// The `session_id` identifies the parent orchestration run in emitted
    /// [`DomainEvent`] payloads. The session starts empty and remains
    /// process-local until a future persistence layer stores snapshots.
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            state: Arc::new(Mutex::new(SessionState::default())),
            notify: Arc::new(Notify::new()),
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
    /// effects include storing a pending snapshot, publishing an
    /// [`DomainEvent::AgentOrchestrationSpawned`] event, emitting parent
    /// progress when available, and waking waiters.
    pub async fn spawn_agent(
        &self,
        request: SpawnAgentRequest,
    ) -> Result<SpawnAgentResponse, OrchestrationError> {
        let parent = current_parent().ok_or(OrchestrationError::NoParentContext)?;
        let definition = resolve_definition(&request)?;
        self.spawn_agent_with_definition(parent, definition, request)
            .await
    }

    /// List all child agents currently known to this session.
    ///
    /// Returns cloned [`AgentSnapshot`] values ordered by creation timestamp.
    /// This method has no fallible paths and does not mutate session state.
    pub async fn list_agents(&self) -> Vec<AgentSnapshot> {
        let state = self.state.lock().await;
        let mut agents: Vec<AgentSnapshot> = state
            .agents
            .values()
            .map(|record| record.snapshot.clone())
            .collect();
        agents.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        agents
    }

    /// Record a parent-to-child message on an orchestration record.
    ///
    /// `request.orchestration_id` selects the child record and
    /// `request.content` must be non-empty after trimming. For a running child,
    /// the snapshot status moves to [`AgentStatus::Waiting`]; terminal children
    /// keep their terminal status so a completed child can still receive a
    /// recorded orchestrator answer before a follow-up child is spawned.
    ///
    /// Returns [`OrchestrationError::InvalidMessage`] for empty content or
    /// [`OrchestrationError::AgentNotFound`] for an unknown child id. Side
    /// effects include appending an [`AgentMessage`], recording a
    /// [`AgentOrchestrationEvent::MessageRecorded`] event, updating the
    /// timestamp, and waking waiters. This records communication metadata only;
    /// it does not inject live input into an already-running harness loop.
    pub async fn message_agent(
        &self,
        request: MessageAgentRequest,
    ) -> Result<AgentSnapshot, OrchestrationError> {
        let content = request.content.trim();
        if content.is_empty() {
            return Err(OrchestrationError::InvalidMessage);
        }

        let mut state = self.state.lock().await;
        let record = state
            .agents
            .get_mut(&request.orchestration_id)
            .ok_or_else(|| OrchestrationError::AgentNotFound(request.orchestration_id.clone()))?;
        if !record.snapshot.status.is_terminal() {
            record.snapshot.status = AgentStatus::Waiting;
        }
        record.snapshot.messages.push(AgentMessage {
            role: "parent".to_string(),
            content: content.to_string(),
            created_at: now(),
        });
        record.snapshot.updated_at = now();
        let snapshot = record.snapshot.clone();
        state.events.push(AgentOrchestrationEvent::MessageRecorded {
            orchestration_id: request.orchestration_id,
        });
        drop(state);
        self.notify.notify_waiters();
        Ok(snapshot)
    }

    /// Wait for one or more child agents to reach terminal status.
    ///
    /// `options.orchestration_ids` names the children to observe. An empty id
    /// list returns the current full session snapshot immediately. When
    /// `timeout_ms` is present, the wait returns a partial response with
    /// `completed = false` after the timeout instead of failing.
    ///
    /// Returns [`OrchestrationError::AgentNotFound`] if any requested child id
    /// is unknown. Side effects are limited to waiting on internal notifications;
    /// no snapshots or events are mutated.
    pub async fn wait_agents(
        &self,
        options: WaitAgentOptions,
    ) -> Result<WaitAgentResponse, OrchestrationError> {
        if options.orchestration_ids.is_empty() {
            return Ok(WaitAgentResponse {
                completed: true,
                agents: self.list_agents().await,
            });
        }

        let wait = async {
            loop {
                let agents = self.snapshots_for(&options.orchestration_ids).await?;
                let completed = agents.iter().all(|agent| agent.status.is_terminal());
                if completed {
                    return Ok(WaitAgentResponse { completed, agents });
                }
                self.notify.notified().await;
            }
        };

        match options.timeout_ms {
            Some(ms) => match timeout(Duration::from_millis(ms), wait).await {
                Ok(response) => response,
                Err(_) => Ok(WaitAgentResponse {
                    completed: false,
                    agents: self.snapshots_for(&options.orchestration_ids).await?,
                }),
            },
            None => wait.await,
        }
    }

    /// Close a child agent record and abort its background task if present.
    ///
    /// `request.orchestration_id` selects the child and `request.reason`, when
    /// present, is stored as the snapshot error/reason. Closing is terminal and
    /// returns the updated snapshot.
    ///
    /// Returns [`OrchestrationError::AgentNotFound`] for an unknown child id.
    /// Side effects include aborting the child task, marking the snapshot as
    /// [`AgentStatus::Closed`], recording an
    /// [`AgentOrchestrationEvent::Closed`] event, publishing
    /// [`DomainEvent::AgentOrchestrationClosed`], and waking waiters.
    pub async fn close_agent(
        &self,
        request: CloseAgentRequest,
    ) -> Result<AgentSnapshot, OrchestrationError> {
        let mut state = self.state.lock().await;
        if let Some(task) = state.tasks.remove(&request.orchestration_id) {
            task.abort();
        }
        let record = state
            .agents
            .get_mut(&request.orchestration_id)
            .ok_or_else(|| OrchestrationError::AgentNotFound(request.orchestration_id.clone()))?;
        record.snapshot.status = AgentStatus::Closed;
        record.snapshot.error = request.reason.clone();
        record.snapshot.updated_at = now();
        let snapshot = record.snapshot.clone();
        state.events.push(AgentOrchestrationEvent::Closed {
            orchestration_id: request.orchestration_id.clone(),
            reason: request.reason.clone(),
        });
        drop(state);
        publish_global(DomainEvent::AgentOrchestrationClosed {
            session_id: self.session_id.clone(),
            orchestration_id: request.orchestration_id,
            reason: request.reason,
        });
        self.notify.notify_waiters();
        Ok(snapshot)
    }

    /// Spawn a linked follow-up child from an existing child record.
    ///
    /// `request.orchestration_id` identifies the previous child and
    /// `request.prompt` is the new delegated instruction. If `request.context`
    /// is absent, the previous child's result summary is used as follow-up
    /// context. The new child inherits the prior child agent id, toolkit, model,
    /// and metadata, and stores `follow_up_of` metadata plus `parent_agent_id`
    /// pointing at the previous child.
    ///
    /// Returns [`OrchestrationError::NoParentContext`] outside an active parent
    /// turn, [`OrchestrationError::AgentNotFound`] for an unknown prior child,
    /// or any spawn-time error from [`Self::spawn_agent`]'s validation path.
    /// Side effects match spawning a new child agent.
    pub async fn follow_up(
        &self,
        request: FollowUpRequest,
    ) -> Result<SpawnAgentResponse, OrchestrationError> {
        let parent = current_parent().ok_or(OrchestrationError::NoParentContext)?;
        let prior = {
            let state = self.state.lock().await;
            state
                .agents
                .get(&request.orchestration_id)
                .cloned()
                .ok_or_else(|| {
                    OrchestrationError::AgentNotFound(request.orchestration_id.clone())
                })?
        };

        let mut metadata = prior.snapshot.metadata.clone();
        metadata.insert(
            "follow_up_of".to_string(),
            prior.snapshot.orchestration_id.clone(),
        );

        let context = request
            .context
            .or_else(|| prior.snapshot.result_summary.clone());
        let spawn = SpawnAgentRequest {
            agent_id: prior.snapshot.agent_id,
            prompt: request.prompt,
            context,
            toolkit: prior.toolkit,
            model: prior.model,
            parent_agent_id: Some(prior.snapshot.orchestration_id),
            metadata,
        };
        let definition = resolve_definition(&spawn)?;
        self.spawn_agent_with_definition(parent, definition, spawn)
            .await
    }

    /// Resume a child by spawning a linked continuation child.
    ///
    /// `request.orchestration_id` selects the prior child. When
    /// `request.prompt` is absent, the previous prompt is reused; the previous
    /// result summary or error becomes the follow-up context. This is a
    /// convenience wrapper around [`Self::follow_up`].
    ///
    /// Returns [`OrchestrationError::AgentNotFound`] if the prior child is
    /// unknown, [`OrchestrationError::NoParentContext`] if no parent turn is
    /// active, or the same spawn-time errors as [`Self::follow_up`]. Side
    /// effects match spawning a linked follow-up child.
    pub async fn resume_agent(
        &self,
        request: ResumeAgentRequest,
    ) -> Result<SpawnAgentResponse, OrchestrationError> {
        let prior = {
            let state = self.state.lock().await;
            state
                .agents
                .get(&request.orchestration_id)
                .map(|record| record.snapshot.clone())
                .ok_or_else(|| {
                    OrchestrationError::AgentNotFound(request.orchestration_id.clone())
                })?
        };
        self.follow_up(FollowUpRequest {
            orchestration_id: request.orchestration_id,
            prompt: request.prompt.unwrap_or(prior.prompt),
            context: prior.result_summary.or(prior.error),
        })
        .await
    }

    /// Return lifecycle events recorded by this session.
    ///
    /// The returned vector is a cloned snapshot of in-memory events in
    /// insertion order. This method has no fallible paths and does not mutate
    /// the session.
    pub async fn events(&self) -> Vec<AgentOrchestrationEvent> {
        self.state.lock().await.events.clone()
    }

    async fn snapshots_for(
        &self,
        ids: &[String],
    ) -> Result<Vec<AgentSnapshot>, OrchestrationError> {
        let state = self.state.lock().await;
        ids.iter()
            .map(|id| {
                state
                    .agents
                    .get(id)
                    .map(|record| record.snapshot.clone())
                    .ok_or_else(|| OrchestrationError::AgentNotFound(id.clone()))
            })
            .collect()
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
        let now = now();
        let snapshot = AgentSnapshot {
            orchestration_id: orchestration_id.clone(),
            agent_id: agent_id.clone(),
            parent_agent_id: request.parent_agent_id.clone(),
            status: AgentStatus::Pending,
            prompt: prompt.clone(),
            messages: Vec::new(),
            result_summary: None,
            error: None,
            created_at: now.clone(),
            updated_at: now,
            metadata: request.metadata.clone(),
        };
        let record = AgentRecord {
            snapshot,
            toolkit: request.toolkit.clone(),
            model: request.model.clone(),
            progress_sink: parent.on_progress.clone(),
        };

        {
            let mut state = self.state.lock().await;
            state.agents.insert(orchestration_id.clone(), record);
            state.events.push(AgentOrchestrationEvent::Spawned {
                orchestration_id: orchestration_id.clone(),
                agent_id: agent_id.clone(),
                parent_agent_id: request.parent_agent_id.clone(),
            });
        }

        publish_global(DomainEvent::AgentOrchestrationSpawned {
            session_id: self.session_id.clone(),
            orchestration_id: orchestration_id.clone(),
            agent_id: agent_id.clone(),
            parent_agent_id: request.parent_agent_id,
        });

        if let Some(progress) = parent.on_progress.clone() {
            let _ = progress
                .send(AgentProgress::SubagentSpawned {
                    agent_id: agent_id.clone(),
                    task_id: orchestration_id.clone(),
                    mode: "typed".to_string(),
                    dedicated_thread: false,
                    prompt_chars: prompt.chars().count(),
                    worker_thread_id: None,
                })
                .await;
        }

        let options = SubagentRunOptions {
            skill_filter_override: None,
            toolkit_override: request.toolkit,
            context: request.context,
            model_override: request.model,
            task_id: Some(orchestration_id.clone()),
            worker_thread_id: None,
        };

        let task_session = self.clone();
        let task_id = orchestration_id.clone();
        let task = tokio::spawn(async move {
            task_session.mark_running(&task_id).await;
            let result = with_parent_context(parent, async move {
                run_subagent(&definition, &prompt, options).await
            })
            .await;
            task_session.finish_agent(&task_id, result).await;
        });

        {
            let mut state = self.state.lock().await;
            state.tasks.insert(orchestration_id.clone(), task);
        }
        self.notify.notify_waiters();

        Ok(SpawnAgentResponse {
            orchestration_id,
            agent_id,
            status: AgentStatus::Pending,
        })
    }

    async fn mark_running(&self, orchestration_id: &str) {
        let mut state = self.state.lock().await;
        if let Some(record) = state.agents.get_mut(orchestration_id) {
            if !record.snapshot.status.is_terminal() {
                record.snapshot.status = AgentStatus::Running;
                record.snapshot.updated_at = now();
            }
        }
        drop(state);
        self.notify.notify_waiters();
    }

    async fn finish_agent(
        &self,
        orchestration_id: &str,
        result: Result<SubagentRunOutcome, crate::openhuman::agent::harness::SubagentRunError>,
    ) {
        let mut completed_event = None;
        let mut failed_event = None;
        let mut progress_event = None;
        let mut state = self.state.lock().await;
        state.tasks.remove(orchestration_id);
        if let Some(record) = state.agents.get_mut(orchestration_id) {
            if record.snapshot.status == AgentStatus::Closed {
                drop(state);
                self.notify.notify_waiters();
                return;
            }
            match result {
                Ok(outcome) => {
                    record.snapshot.status = AgentStatus::Completed;
                    record.snapshot.result_summary = Some(outcome.output.clone());
                    record.snapshot.updated_at = now();
                    let event = AgentOrchestrationEvent::Completed {
                        orchestration_id: orchestration_id.to_string(),
                        output_chars: outcome.output.chars().count(),
                        iterations: outcome.iterations,
                    };
                    if let Some(progress) = record.progress_sink.clone() {
                        progress_event = Some((
                            progress,
                            AgentProgress::SubagentCompleted {
                                agent_id: outcome.agent_id.clone(),
                                task_id: orchestration_id.to_string(),
                                elapsed_ms: outcome.elapsed.as_millis() as u64,
                                iterations: outcome.iterations as u32,
                                output_chars: outcome.output.chars().count(),
                            },
                        ));
                    }
                    completed_event = Some((outcome, event.clone()));
                    state.events.push(event);
                }
                Err(error) => {
                    let message = error.to_string();
                    record.snapshot.status = AgentStatus::Failed;
                    record.snapshot.error = Some(message.clone());
                    record.snapshot.updated_at = now();
                    let event = AgentOrchestrationEvent::Failed {
                        orchestration_id: orchestration_id.to_string(),
                        error: message.clone(),
                    };
                    if let Some(progress) = record.progress_sink.clone() {
                        progress_event = Some((
                            progress,
                            AgentProgress::SubagentFailed {
                                agent_id: record.snapshot.agent_id.clone(),
                                task_id: orchestration_id.to_string(),
                                error: message.clone(),
                            },
                        ));
                    }
                    failed_event = Some((record.snapshot.agent_id.clone(), message, event.clone()));
                    state.events.push(event);
                }
            }
        }
        drop(state);

        if let Some((outcome, _)) = completed_event {
            publish_global(DomainEvent::AgentOrchestrationCompleted {
                session_id: self.session_id.clone(),
                orchestration_id: orchestration_id.to_string(),
                agent_id: outcome.agent_id,
                elapsed_ms: outcome.elapsed.as_millis() as u64,
                output_chars: outcome.output.chars().count(),
                iterations: outcome.iterations,
            });
        }
        if let Some((agent_id, error, _)) = failed_event {
            publish_global(DomainEvent::AgentOrchestrationFailed {
                session_id: self.session_id.clone(),
                orchestration_id: orchestration_id.to_string(),
                agent_id,
                error,
            });
        }
        if let Some((progress, event)) = progress_event {
            let _ = progress.send(event).await;
        }
        self.notify.notify_waiters();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_statuses_are_explicit() {
        assert!(AgentStatus::Completed.is_terminal());
        assert!(AgentStatus::Failed.is_terminal());
        assert!(AgentStatus::Cancelled.is_terminal());
        assert!(AgentStatus::Closed.is_terminal());
        assert!(!AgentStatus::Pending.is_terminal());
        assert!(!AgentStatus::Running.is_terminal());
        assert!(!AgentStatus::Waiting.is_terminal());
    }

    #[tokio::test]
    async fn empty_wait_lists_current_agents() {
        let session = AgentOrchestrationSession::new("test-session");
        let response = session
            .wait_agents(WaitAgentOptions {
                orchestration_ids: Vec::new(),
                timeout_ms: Some(1),
            })
            .await
            .unwrap();

        assert!(response.completed);
        assert!(response.agents.is_empty());
    }

    #[tokio::test]
    async fn unknown_wait_target_returns_not_found() {
        let session = AgentOrchestrationSession::new("test-session");
        let err = session
            .wait_agents(WaitAgentOptions {
                orchestration_ids: vec!["missing".to_string()],
                timeout_ms: Some(1),
            })
            .await
            .unwrap_err();

        assert!(matches!(err, OrchestrationError::AgentNotFound(id) if id == "missing"));
    }
}
