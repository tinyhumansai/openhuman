//! Adapter from TinyAgents' durable subagent-session port to OpenHuman storage.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tinyagents_orchestration::subagent::{
    DriverSubagentSessionStore, DurableSubagentSession as TinySession,
    DurableSubagentStatus as TinyStatus, ReuseDecision as TinyReuseDecision,
    ScopedSubagentSessionStore, SubagentError, SubagentSessionSelector as TinySelector,
    SubagentSessionStore as TinySessionStore, SubagentSessionUpsert as TinyUpsert,
};

use super::tiny_port_epoch::import_parent_epoch;
#[cfg(test)]
use super::tiny_port_epoch::parent_epoch_from_sidecar;
use super::types::{DurableSubagentSession, DurableSubagentStatus, SubagentSessionSelector};
use super::{SubagentSessionStore, SubagentSessionUpsert};

pub(crate) fn host_projection(
    session: TinySession,
    parent_epoch: Option<u64>,
) -> DurableSubagentSession {
    DurableSubagentSession {
        subagent_session_id: session.subagent_session_id,
        parent_session: session.parent_session,
        parent_thread_id: session.parent_thread_id,
        worker_thread_id: session.worker_thread_id,
        agent_id: session.agent_id,
        display_name: session.display_name,
        model: session.model,
        sandbox_mode: session.sandbox_mode,
        action_root: session.action_root,
        task_key: session.task_key,
        task_title: session.task_title,
        current_task_id: session.current_task_id,
        lifecycle_key: session.lifecycle_key,
        parent_epoch,
        status: match session.status {
            TinyStatus::Running => DurableSubagentStatus::Running,
            TinyStatus::Idle => DurableSubagentStatus::Idle,
            TinyStatus::AwaitingUser => DurableSubagentStatus::AwaitingUser,
            TinyStatus::Failed => DurableSubagentStatus::Failed,
            TinyStatus::Closed => DurableSubagentStatus::Closed,
        },
        reusable: session.reusable,
        latest_history: session.latest_history.map(|history| {
            tinyagents_runtime::transcript::messages_to_display_transcript(&history)
        }),
        latest_error: session.latest_error,
        created_at: session.created_at,
        updated_at: session.updated_at,
        last_used_at: session.last_used_at,
    }
}

/// Opens TinyAgents' async session store in the caller's explicit storage
/// scope and binds it to one parent session/thread. The core's storage handle
/// is captured before returning, so later trait calls never consult ambient
/// task-local scope.
pub(crate) fn open_for_context(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace_dir: PathBuf,
    parent_session: impl Into<String>,
    parent_thread_id: Option<String>,
    jobs: Option<tinyagents_orchestration::subagent::SubAgentJobRegistry>,
) -> Result<Arc<dyn TinySessionStore>, SubagentError> {
    let documents = documents_for_context(ctx, &workspace_dir)?;
    let driver = Arc::new(DriverSubagentSessionStore::new(Arc::clone(&documents))?);
    let parent_session = parent_session.into();
    import_host_sessions(
        ctx,
        &workspace_dir,
        &parent_session,
        parent_thread_id.as_deref(),
        documents,
    )?;
    let owner = crate::agent::tinyagents::tool_registration::subagent_owner_key(
        &workspace_dir,
        &parent_session,
        parent_thread_id.as_deref(),
    );
    let scoped: Arc<dyn TinySessionStore> = if let Some(jobs) = jobs {
        Arc::new(ScopedSubagentSessionStore::with_close_callback(
            Arc::clone(&driver),
            parent_session.clone(),
            parent_thread_id.clone(),
            move |_session_id, _task_id, key| {
                if let Err(error) = jobs.cancel_key_owned(key, owner.clone()) {
                    log::debug!("[subagent_sessions] close cancellation was not applied: {error}");
                }
            },
        ))
    } else {
        Arc::new(ScopedSubagentSessionStore::new(
            Arc::clone(&driver),
            parent_session.clone(),
            parent_thread_id.clone(),
        ))
    };
    Ok(Arc::new(TaskIdSessionLookup {
        inner: scoped,
        parent_session,
        parent_thread_id,
    }))
}

/// Imports one terminal legacy task into TinyAgents' scoped session store and
/// returns the port's view of it. The legacy read only discovers the parent
/// scope; all terminal reporting is based on the migrated driver row.
pub(crate) async fn import_terminal_task_for_context(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace_dir: PathBuf,
    task_id: &str,
) -> Result<Option<TinySession>, SubagentError> {
    let legacy = SubagentSessionStore::new_in_context(ctx, workspace_dir.clone())
        .map_err(SubagentError::Persistence)?
        .find_task_for_import(task_id)
        .map_err(SubagentError::Persistence)?;
    let Some(row) = legacy.filter(|row| {
        matches!(
            row.status,
            DurableSubagentStatus::Idle | DurableSubagentStatus::Failed
        )
    }) else {
        return Ok(None);
    };
    let parent_session = row.parent_session.clone();
    let parent_thread_id = row.parent_thread_id.clone();
    let documents = documents_for_context(ctx, &workspace_dir)?;
    import_host_session(
        &workspace_dir,
        &parent_session,
        parent_thread_id.as_deref(),
        row,
        documents.clone(),
    )?;
    let driver = DriverSubagentSessionStore::new(documents)?;
    let session = driver
        .list_for_parent(&parent_session, parent_thread_id.as_deref())
        .await?
        .into_iter()
        .find(|session| session.current_task_id.as_deref() == Some(task_id));
    Ok(session)
}

fn documents_for_context(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace_dir: &std::path::Path,
) -> Result<Arc<dyn crate::storage::DocumentStore>, SubagentError> {
    let open_documents = || {
        let opened =
            crate::storage::local::open("subagent_sessions", workspace_dir).map_err(|error| {
                SubagentError::Persistence(format!("open session store: {error:#}"))
            })?;
        let scoped = opened.scoped().map_err(|error| {
            SubagentError::Persistence(format!("scope session store: {error:#}"))
        })?;
        Ok::<_, SubagentError>(Arc::clone(scoped.documents()))
    };
    let context = ctx.resolved().ok_or_else(|| {
        SubagentError::Persistence("subagent session store has no resolved owner scope".into())
    })?;
    crate::core::runtime::CoreContext::sync_scope(context, open_documents)
}

/// Copies the pre-driver host collection into TinyAgents' collection without
/// removing or rewriting the source rows. The old collection remains the
/// owner of OpenHuman-only `parent_epoch` until the host's epoch sidecar is
/// established independently.
fn import_host_sessions(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace_dir: &std::path::Path,
    parent_session: &str,
    parent_thread_id: Option<&str>,
    documents: Arc<dyn crate::storage::DocumentStore>,
) -> Result<(), SubagentError> {
    let legacy = SubagentSessionStore::new_in_context(ctx, workspace_dir.to_path_buf())
        .map_err(SubagentError::Persistence)?
        .load()
        .map_err(SubagentError::Persistence)?;
    for row in legacy.into_iter().filter(|row| {
        row.parent_session == parent_session && row.parent_thread_id.as_deref() == parent_thread_id
    }) {
        import_host_session(
            workspace_dir,
            parent_session,
            parent_thread_id,
            row,
            Arc::clone(&documents),
        )?;
    }
    Ok(())
}

fn import_host_session(
    workspace_dir: &std::path::Path,
    parent_session: &str,
    parent_thread_id: Option<&str>,
    row: super::DurableSubagentSession,
    documents: Arc<dyn crate::storage::DocumentStore>,
) -> Result<(), SubagentError> {
    import_parent_epoch(
        Arc::clone(&documents),
        row.subagent_session_id.clone(),
        row.parent_epoch,
    )?;
    let converter = OpenHumanTinySubagentSessionStore::new(
        workspace_dir.to_path_buf(),
        parent_session,
        parent_thread_id.map(str::to_owned),
    );
    let session = converter.convert(row)?;
    let id = session.subagent_session_id.clone();
    let docs = Arc::clone(&documents);
    let existing = crate::storage::block_on(async move {
        docs.ensure_collection(&tinystoragedrivers::CollectionSpec::new(
            "subagent_sessions",
        ))
        .await?;
        docs.get("subagent_sessions", &id).await
    })
    .map_err(|error| SubagentError::Persistence(format!("read driver session: {error}")))?;
    if existing.is_some() {
        return Ok(());
    }
    // The driver's upsert API intentionally only creates a Running row;
    // import existing terminal/reusable state as its public durable shape.
    let doc = serde_json::to_value(&session)
        .map_err(|error| SubagentError::Persistence(format!("session encode: {error}")))?;
    let write_docs = Arc::clone(&documents);
    crate::storage::block_on(async move {
        write_docs
            .ensure_collection(&tinystoragedrivers::CollectionSpec::new(
                "subagent_sessions",
            ))
            .await?;
        match write_docs
            .put(
                "subagent_sessions",
                &session.subagent_session_id,
                doc,
                crate::storage::Precondition::Absent,
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(error) if error.kind() == tinystoragedrivers::ErrorKind::Conflict => Ok(()),
            Err(error) => Err(error),
        }
    })
    .map_err(|error| SubagentError::Persistence(format!("import host session: {error}")))
}

/// Host compatibility seam: public orchestration tools historically accept a
/// task id as well as a durable session id. TinyAgents owns the durable store
/// and owner checks; this adapter only resolves that task-id alias in the
/// already owner-scoped row set.
struct TaskIdSessionLookup {
    inner: Arc<dyn TinySessionStore>,
    parent_session: String,
    parent_thread_id: Option<String>,
}

#[async_trait]
impl TinySessionStore for TaskIdSessionLookup {
    async fn list_for_parent(
        &self,
        parent_session: &str,
        parent_thread_id: Option<&str>,
    ) -> Result<Vec<TinySession>, SubagentError> {
        self.inner
            .list_for_parent(parent_session, parent_thread_id)
            .await
    }

    async fn get(&self, id: &str) -> Result<Option<TinySession>, SubagentError> {
        if let Some(session) = self.inner.get(id).await? {
            return Ok(Some(session));
        }
        let mut matches = self
            .inner
            .list_for_parent(&self.parent_session, self.parent_thread_id.as_deref())
            .await?
            .into_iter()
            .filter(|session| session.current_task_id.as_deref() == Some(id));
        let session = matches.next();
        if matches.next().is_some() {
            return Err(SubagentError::InvalidRequest(
                "subagent task id is ambiguous within the authenticated owner scope".into(),
            ));
        }
        Ok(session)
    }

    async fn upsert_running(
        &self,
        upsert: TinyUpsert,
    ) -> Result<(TinySession, TinyReuseDecision), SubagentError> {
        self.inner.upsert_running(upsert).await
    }

    async fn close(&self, id: &str) -> Result<bool, SubagentError> {
        self.inner.close(id).await
    }

    async fn close_if_current(
        &self,
        id: &str,
        task_id: &str,
        lifecycle_key: &tinyagents_orchestration::subagent::SubagentTaskKey,
        parent_session: &str,
        parent_thread_id: Option<&str>,
    ) -> Result<bool, SubagentError> {
        self.inner
            .close_if_current(id, task_id, lifecycle_key, parent_session, parent_thread_id)
            .await
    }

    async fn mark_finished(
        &self,
        id: &str,
        task_id: &str,
        status: TinyStatus,
        history: Vec<tinyinference_llm::message::Message>,
    ) -> Result<bool, SubagentError> {
        self.inner.mark_finished(id, task_id, status, history).await
    }

    async fn mark_finished_if_current(
        &self,
        id: &str,
        task_id: &str,
        lifecycle_key: &tinyagents_orchestration::subagent::SubagentTaskKey,
        parent_session: &str,
        parent_thread_id: Option<&str>,
        status: TinyStatus,
        history: Vec<tinyinference_llm::message::Message>,
    ) -> Result<bool, SubagentError> {
        self.inner
            .mark_finished_if_current(
                id,
                task_id,
                lifecycle_key,
                parent_session,
                parent_thread_id,
                status,
                history,
            )
            .await
    }

    async fn mark_failed(
        &self,
        id: &str,
        task_id: &str,
        error: String,
    ) -> Result<bool, SubagentError> {
        self.inner.mark_failed(id, task_id, error).await
    }

    async fn mark_failed_if_current(
        &self,
        id: &str,
        task_id: &str,
        lifecycle_key: &tinyagents_orchestration::subagent::SubagentTaskKey,
        parent_session: &str,
        parent_thread_id: Option<&str>,
        error: String,
    ) -> Result<bool, SubagentError> {
        self.inner
            .mark_failed_if_current(
                id,
                task_id,
                lifecycle_key,
                parent_session,
                parent_thread_id,
                error,
            )
            .await
    }
}

/// Routes generic Jobs/Message session operations through OpenHuman's existing
/// workspace-scoped session store. Legacy rows acquire a lifecycle key only
/// from their validated pause checkpoint; the public task id is never enough.
/// The current TinyAgents session upsert contract does not carry its driver's
/// admitted parent epoch, so these rows retain `None` and use the completion
/// router's legacy epoch-zero fencing semantics.
#[derive(Clone)]
pub(crate) struct OpenHumanTinySubagentSessionStore {
    store: SubagentSessionStore,
    checkpoint_dir: PathBuf,
    parent_session: String,
    parent_thread_id: Option<String>,
    jobs: Option<tinyagents_orchestration::subagent::SubAgentJobRegistry>,
}

impl OpenHumanTinySubagentSessionStore {
    pub(crate) fn new(
        workspace_dir: PathBuf,
        parent_session: impl Into<String>,
        parent_thread_id: Option<String>,
    ) -> Self {
        Self {
            checkpoint_dir: workspace_dir.join(".openhuman/subagent_checkpoints"),
            store: SubagentSessionStore::new(workspace_dir),
            parent_session: parent_session.into(),
            parent_thread_id,
            jobs: None,
        }
    }

    pub(crate) fn with_jobs(
        mut self,
        jobs: tinyagents_orchestration::subagent::SubAgentJobRegistry,
    ) -> Self {
        self.jobs = Some(jobs);
        self
    }

    fn is_owned(&self, row: &DurableSubagentSession) -> bool {
        row.parent_session == self.parent_session && row.parent_thread_id == self.parent_thread_id
    }

    fn convert(&self, row: DurableSubagentSession) -> Result<TinySession, SubagentError> {
        let lifecycle_key = match (row.lifecycle_key.clone(), row.current_task_id.as_deref()) {
            (Some(key), Some(task_id))
                if key.task_id == task_id
                    && key.thread_id.as_deref() == row.parent_thread_id.as_deref() =>
            {
                Some(key)
            }
            (Some(_), _) => None,
            (None, Some(task_id)) => super::resolve_lifecycle_key(
                &self.store,
                &row.subagent_session_id,
                &row.parent_session,
                row.parent_thread_id.as_deref(),
                &self.checkpoint_dir,
            )
            .ok()
            .filter(|key| key.task_id == task_id),
            // Keep a pre-lifecycle row visible to list/query. Its missing key
            // is not reconstructed from the raw task id, and continuation by
            // session id remains fail-closed in TinyAgents.
            (None, None) => None,
        };
        Ok(TinySession {
            subagent_session_id: row.subagent_session_id,
            parent_session: row.parent_session,
            parent_thread_id: row.parent_thread_id,
            worker_thread_id: row.worker_thread_id,
            agent_id: row.agent_id,
            display_name: row.display_name,
            model: row.model,
            sandbox_mode: row.sandbox_mode,
            action_root: row.action_root,
            task_key: row.task_key,
            lifecycle_key,
            task_title: row.task_title,
            current_task_id: row.current_task_id,
            status: match row.status {
                DurableSubagentStatus::Running => TinyStatus::Running,
                DurableSubagentStatus::Idle => TinyStatus::Idle,
                DurableSubagentStatus::AwaitingUser => TinyStatus::AwaitingUser,
                DurableSubagentStatus::Failed => TinyStatus::Failed,
                DurableSubagentStatus::Closed => TinyStatus::Closed,
            },
            reusable: row.reusable,
            latest_history: row.latest_history.map(|history| {
                history
                    .iter()
                    .map(tinyagents_runtime::transcript::transcript_to_message)
                    .collect()
            }),
            latest_error: row.latest_error,
            created_at: row.created_at,
            updated_at: row.updated_at,
            last_used_at: row.last_used_at,
        })
    }

    fn selector(selector: TinySelector) -> SubagentSessionSelector {
        SubagentSessionSelector {
            parent_session: selector.parent_session,
            parent_thread_id: selector.parent_thread_id,
            agent_id: selector.agent_id,
            model: selector.model,
            sandbox_mode: selector.sandbox_mode,
            action_root: selector.action_root,
            task_key: selector.task_key,
        }
    }
}

#[async_trait]
impl TinySessionStore for OpenHumanTinySubagentSessionStore {
    async fn list_for_parent(
        &self,
        parent_session: &str,
        parent_thread_id: Option<&str>,
    ) -> Result<Vec<TinySession>, SubagentError> {
        if parent_session != self.parent_session {
            return Ok(Vec::new());
        }
        if parent_thread_id
            .is_some_and(|requested| self.parent_thread_id.as_deref() != Some(requested))
        {
            return Ok(Vec::new());
        }
        super::list_for_parent(
            &self.store,
            parent_session,
            self.parent_thread_id.as_deref(),
        )
        .map_err(SubagentError::Persistence)?
        .into_iter()
        .filter(|row| self.is_owned(row))
        .map(|row| self.convert(row))
        .collect()
    }

    async fn get(&self, id: &str) -> Result<Option<TinySession>, SubagentError> {
        let mut owned_rows = self
            .store
            .load()
            .map_err(SubagentError::Persistence)?
            .into_iter()
            // Filter the authenticated parent and thread before matching either
            // public identifier. Raw task IDs can collide across owners, so a
            // foreign row must never shadow a matching owned row.
            .filter(|row| self.is_owned(row))
            .collect::<Vec<_>>();

        // Product callers historically pass the durable session ID, while
        // accepted spawn results also expose `current_task_id`. Prefer the
        // unambiguous session key when both happen to match.
        let row = if let Some(index) = owned_rows
            .iter()
            .position(|row| row.subagent_session_id == id)
        {
            Some(owned_rows.remove(index))
        } else {
            let mut matches = owned_rows
                .into_iter()
                .filter(|row| row.current_task_id.as_deref() == Some(id));
            let row = matches.next();
            if matches.next().is_some() {
                return Err(SubagentError::InvalidRequest(
                    "subagent task id is ambiguous within the authenticated owner scope".into(),
                ));
            }
            row
        };

        row.map(|row| self.convert(row)).transpose()
    }

    async fn upsert_running(
        &self,
        upsert: TinyUpsert,
    ) -> Result<(TinySession, TinyReuseDecision), SubagentError> {
        if upsert.selector.parent_session != self.parent_session
            || upsert.selector.parent_thread_id != self.parent_thread_id
            || upsert.lifecycle_key.thread_id != self.parent_thread_id
            || upsert.lifecycle_key.task_id != upsert.task_id
        {
            return Err(SubagentError::InvalidRequest(
                "subagent session upsert is outside its authenticated owner scope".into(),
            ));
        }
        let selector = Self::selector(upsert.selector);
        let reuse = if upsert.force_fresh {
            None
        } else {
            super::find_reusable(&self.store, &selector).map_err(SubagentError::Persistence)?
        };
        let decision = if upsert.force_fresh {
            TinyReuseDecision::ForcedFresh
        } else if let Some(session) = reuse.as_ref() {
            if session.status == DurableSubagentStatus::Running {
                TinyReuseDecision::ReusedRunning
            } else {
                TinyReuseDecision::ReusedIdle
            }
        } else {
            TinyReuseDecision::SpawnedNew
        };
        let row = super::upsert_running(
            &self.store,
            SubagentSessionUpsert {
                selector,
                display_name: upsert.display_name,
                task_title: upsert.task_title,
                worker_thread_id: upsert.worker_thread_id,
                task_id: upsert.task_id,
                lifecycle_key: Some(upsert.lifecycle_key),
                // TinyUpsert has no admitted parent epoch. Do not infer one
                // from the live router here; admission happened separately in
                // the owning driver and a later lifecycle could now be open.
                parent_epoch: None,
            },
            reuse.as_ref(),
        )
        .map_err(SubagentError::Persistence)?;
        Ok((self.convert(row)?, decision))
    }

    async fn close(&self, id: &str) -> Result<bool, SubagentError> {
        let session = self
            .store
            .load()
            .map_err(SubagentError::Persistence)?
            .into_iter()
            .find(|session| session.subagent_session_id == id && self.is_owned(session));
        let Some(session) = session else {
            return Ok(false);
        };
        if session.status == DurableSubagentStatus::Closed {
            return super::close_if_current(&self.store, &session)
                .map_err(SubagentError::Persistence);
        }
        if let Some(jobs) = &self.jobs {
            let _ = super::cancel_driver_job_for_observed_session(
                &self.store,
                &self.checkpoint_dir,
                jobs,
                &session,
                &self.parent_session,
                self.parent_thread_id.as_deref(),
            );
        }
        match super::close_if_current(&self.store, &session).map_err(SubagentError::Persistence)? {
            true => Ok(true),
            false => Err(SubagentError::InvalidRequest(
                "subagent session changed while it was being closed".into(),
            )),
        }
    }

    async fn mark_finished(
        &self,
        id: &str,
        task_id: &str,
        status: TinyStatus,
        history: Vec<tinyinference_llm::message::Message>,
    ) -> Result<bool, SubagentError> {
        if self.get(id).await?.is_none() {
            return Ok(false);
        }
        let status = match status {
            TinyStatus::Running => crate::agent::subagent_host::SubagentRunStatus::Incomplete {
                reason: "still running".into(),
                failure_cause: None,
            },
            TinyStatus::Idle => crate::agent::subagent_host::SubagentRunStatus::Completed,
            TinyStatus::AwaitingUser => {
                crate::agent::subagent_host::SubagentRunStatus::AwaitingUser {
                    question: history
                        .last()
                        .map(tinyinference_llm::message::Message::text)
                        .unwrap_or_default(),
                    options: None,
                    checkpoint: Some(self.checkpoint_dir.clone()),
                }
            }
            TinyStatus::Failed => crate::agent::subagent_host::SubagentRunStatus::Cancelled,
            TinyStatus::Closed => return self.close(id).await,
        };
        let history = history
            .iter()
            .filter_map(tinyagents_runtime::transcript::message_to_native_transcript)
            .collect();
        super::mark_finished(&self.store, id, task_id, &status, history)
            .map_err(SubagentError::Persistence)?;
        Ok(true)
    }

    async fn mark_failed(
        &self,
        id: &str,
        task_id: &str,
        error: String,
    ) -> Result<bool, SubagentError> {
        if self.get(id).await?.is_none() {
            return Ok(false);
        }
        super::mark_failed(&self.store, id, task_id, error).map_err(SubagentError::Persistence)?;
        Ok(true)
    }
}

#[cfg(test)]
#[path = "tiny_port_tests.rs"]
mod tests;
