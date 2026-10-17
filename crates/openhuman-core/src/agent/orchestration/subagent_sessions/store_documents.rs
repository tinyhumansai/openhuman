//! Durable sub-agent sessions on the `tinystoragedrivers` document port.
//!
//! The only store: sessions live under the acting agent's storage scope
//! (`local` on a single-user host) in the backend the host installed
//! ([`crate::storage`]), or the workspace default when none is installed
//! ([`crate::storage::local`]). `store.rs` opens it per call.
//!
//! # Layout
//!
//! | Collection | Document id | Holds |
//! | --- | --- | --- |
//! | `subagents.sessions` | `Key::of([session id])` | one [`DurableSubagentSession`] as a snake_case [`StoredSession`] with `_ms` times |
//!
//! The one-time import of the legacy `subagent_sessions.json` is recorded in
//! `tsd.migrations` ([`crate::storage::import_state`]), not in a marker
//! collection, so `load` never sees it as a session. Shapes and the v1 -> v2
//! step are in [`super::storage_schema`].
//!
//! The callers in `ops.rs` load and save the whole list. `save` writes each session as
//! its own document, so a session's history stays well under any document-size
//! limit and two processes only collide on a session they both changed. `save`
//! never deletes: `ops.rs` only replaces a session (remove then re-add the same
//! id) and never drops one, and a delete sweep over a stale snapshot would
//! remove sessions another process added meanwhile.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tinyagents_session::transcript::TranscriptMessage;
use tinystoragedrivers::{CollectionSpec, Query, Sort};

use super::storage_schema::{spec, SESSIONS};
use super::types::{DurableSubagentSession, DurableSubagentStatus};
use crate::storage::documents::Repo;
use crate::storage::import_state;
use crate::storage::schema_support::{key_of, ms_rfc3339, rfc3339_ms};
use crate::storage::DocumentStoreExt;

/// The `tsd.migrations` owner of the legacy-file import.
const OWNER: &str = "subagents";
/// The old marker collection, now the `kind` of the import's step.
const IMPORT_KIND: &str = "subagent_sessions_import";
const LEGACY_MARKER: &str = "subagent_sessions.json";

/// The step id for one workspace's legacy file, so workspaces sharing a
/// backend each import their own.
fn import_step(source: &std::path::Path) -> String {
    import_state::step_id(
        IMPORT_KIND,
        &format!("{LEGACY_MARKER}:{}", crate::storage::source_key(source)),
    )
}
const DOMAIN: &str = "subagent_sessions::store";

fn collections() -> Vec<CollectionSpec> {
    vec![spec()]
}

/// A session as stored: snake_case, `_ms` times. The RPC shape
/// ([`DurableSubagentSession`]) is camelCase with RFC 3339 strings.
#[derive(Serialize, Deserialize)]
struct StoredSession {
    subagent_session_id: String,
    parent_session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    worker_thread_id: Option<String>,
    agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    sandbox_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    action_root: Option<String>,
    task_key: String,
    task_title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    current_task_id: Option<String>,
    status: DurableSubagentStatus,
    reusable: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::agent::messages::history_wire::option"
    )]
    latest_history: Option<Vec<TranscriptMessage>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latest_error: Option<String>,
    created_ms: i64,
    updated_ms: i64,
    last_used_ms: i64,
}

impl From<&DurableSubagentSession> for StoredSession {
    fn from(s: &DurableSubagentSession) -> Self {
        let created_ms = rfc3339_ms(&s.created_at).unwrap_or_default();
        let updated_ms = rfc3339_ms(&s.updated_at).unwrap_or(created_ms);
        Self {
            subagent_session_id: s.subagent_session_id.clone(),
            parent_session: s.parent_session.clone(),
            parent_thread_id: s.parent_thread_id.clone(),
            worker_thread_id: s.worker_thread_id.clone(),
            agent_id: s.agent_id.clone(),
            display_name: s.display_name.clone(),
            model: s.model.clone(),
            sandbox_mode: s.sandbox_mode.clone(),
            action_root: s.action_root.clone(),
            task_key: s.task_key.clone(),
            task_title: s.task_title.clone(),
            current_task_id: s.current_task_id.clone(),
            status: s.status,
            reusable: s.reusable,
            latest_history: s.latest_history.clone(),
            latest_error: s.latest_error.clone(),
            created_ms,
            updated_ms,
            last_used_ms: rfc3339_ms(&s.last_used_at).unwrap_or(updated_ms),
        }
    }
}

impl From<StoredSession> for DurableSubagentSession {
    fn from(s: StoredSession) -> Self {
        Self {
            subagent_session_id: s.subagent_session_id,
            parent_session: s.parent_session,
            parent_thread_id: s.parent_thread_id,
            worker_thread_id: s.worker_thread_id,
            agent_id: s.agent_id,
            display_name: s.display_name,
            model: s.model,
            sandbox_mode: s.sandbox_mode,
            action_root: s.action_root,
            task_key: s.task_key,
            task_title: s.task_title,
            current_task_id: s.current_task_id,
            status: s.status,
            reusable: s.reusable,
            latest_history: s.latest_history,
            latest_error: s.latest_error,
            created_at: ms_rfc3339(s.created_ms),
            updated_at: ms_rfc3339(s.updated_ms),
            last_used_at: ms_rfc3339(s.last_used_ms),
        }
    }
}

/// The document store for this call: the installed backend, else the
/// workspace default, in the acting scope.
pub(super) fn current(workspace_dir: &std::path::Path) -> Result<Docs> {
    #[cfg(test)]
    if let Some(docs) = TEST_OVERRIDE.with(|slot| slot.borrow().clone()) {
        return Ok(docs);
    }
    Ok(Docs(Repo::current_for(workspace_dir, DOMAIN, collections)?))
}

/// Runs `f` with `docs` standing in for the installed backend, on this thread
/// only, so tests exercise the dispatch without the process-wide slot.
#[cfg(test)]
pub(super) fn with_override<T>(docs: Docs, f: impl FnOnce() -> T) -> T {
    TEST_OVERRIDE.with(|slot| *slot.borrow_mut() = Some(docs));
    let out = f();
    TEST_OVERRIDE.with(|slot| *slot.borrow_mut() = None);
    out
}

#[cfg(test)]
thread_local! {
    static TEST_OVERRIDE: std::cell::RefCell<Option<Docs>> = const { std::cell::RefCell::new(None) };
}

/// The session list over one scoped document handle.
#[derive(Clone)]
pub(super) struct Docs(Repo);

impl Docs {
    #[cfg(test)]
    pub(super) fn over(scoped: &crate::storage::ScopedStorage) -> Self {
        Self(Repo::over(scoped, DOMAIN, collections))
    }

    pub(super) fn load(&self) -> Result<Vec<DurableSubagentSession>> {
        let stored = self.0.run(|docs| async move {
            docs.query_all(
                SESSIONS,
                &Query::all()
                    .sort(Sort::asc("created_ms"))
                    .sort(Sort::asc("_id")),
            )
            .await
        })?;
        stored
            .into_iter()
            .map(|item| Ok(serde_json::from_value::<StoredSession>(item.doc)?.into()))
            .collect()
    }

    /// Whether the one-time legacy file import finished in this scope.
    pub(super) fn legacy_imported(&self, source: &std::path::Path) -> Result<bool> {
        import_state::is_done(&self.0, OWNER, &import_step(source))
    }

    /// Imports `sessions` read from the legacy file (a stored newer copy
    /// wins), then records the import in `tsd.migrations`.
    pub(super) fn import_legacy(
        &self,
        source: &std::path::Path,
        sessions: &[DurableSubagentSession],
    ) -> Result<()> {
        let step = import_step(source);
        let claimed = import_state::with_claim(&self.0, OWNER, &step, || {
            // The process that held the claim before us may have finished.
            if self.legacy_imported(source)? {
                return Ok(());
            }
            self.save(sessions)?;
            import_state::set_done(&self.0, OWNER, &step)
        })?;
        if claimed.is_none() {
            log::debug!(
                "[subagent_sessions] another process is importing the legacy file; skipped"
            );
        }
        Ok(())
    }

    pub(super) fn save(&self, sessions: &[DurableSubagentSession]) -> Result<()> {
        let mut writes = Vec::with_capacity(sessions.len());
        for session in sessions {
            let stored = StoredSession::from(session);
            let updated = stored.updated_ms;
            let id = key_of(&session.subagent_session_id)?;
            writes.push((id, serde_json::to_value(&stored)?, updated));
        }
        log::debug!("[subagent_sessions] document save count={}", writes.len());
        self.0.run(|docs| async move {
            for (id, doc, updated) in writes {
                write_if_newer(&docs, &id, doc, updated).await?;
            }
            Ok(())
        })
    }
}

/// Writes one session unless the stored copy is at least as new, under
/// compare-and-swap. A caller holds a snapshot of the whole list, so a
/// session it did not change is older than (or equal to) what another core may
/// have stored since, and must not be put back.
async fn write_if_newer(
    docs: &std::sync::Arc<dyn crate::storage::DocumentStore>,
    id: &str,
    doc: Value,
    updated_ms: i64,
) -> Result<(), crate::storage::StorageError> {
    // A missing session is inserted; an existing one is replaced only while it
    // is older than ours. Both go through tsd's one retrying compare-and-swap.
    let repo = tinystoragedrivers::Repo::<Value>::new(docs, SESSIONS)?;
    repo.update(id, |stored| {
        if stored
            .as_ref()
            .and_then(|stored| stored.get("updated_ms"))
            .and_then(Value::as_i64)
            .is_some_and(|have| have >= updated_ms)
        {
            return Ok(None);
        }
        Ok(Some(doc.clone()))
    })
    .await
    .map(|_| ())
}

#[cfg(test)]
#[path = "store_documents_tests.rs"]
mod tests;
