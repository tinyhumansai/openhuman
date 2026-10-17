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
//! | `subagent_sessions` | the session id | one [`DurableSubagentSession`] plus `created_at` for ordering |
//! | `subagent_sessions_import` | `subagent_sessions.json:<workspace key>` | marker: the legacy file was imported (it is left byte-identical) |
//!
//! The callers in `ops.rs` load and save the whole list. `save` writes each session as
//! its own document, so a session's history stays well under any document-size
//! limit and two processes only collide on a session they both changed. `save`
//! never deletes: `ops.rs` only replaces a session (remove then re-add the same
//! id) and never drops one, and a delete sweep over a stale snapshot would
//! remove sessions another process added meanwhile.

use anyhow::Result;
use serde_json::Value;
use tinystoragedrivers::{CollectionSpec, Precondition, Query, Sort};

use super::types::DurableSubagentSession;
use crate::storage::documents::Repo;
use crate::storage::DocumentStoreExt;

const SESSIONS: &str = "subagent_sessions";
/// Holds the marker document that records the one-time legacy file import, in
/// a collection of its own so `load` never sees it as a session.
const IMPORTED: &str = "subagent_sessions_import";
const LEGACY_MARKER: &str = "subagent_sessions.json";

/// The marker id for one workspace's legacy file, so workspaces sharing a
/// backend each import their own.
fn marker_id(source: &std::path::Path) -> String {
    format!("{LEGACY_MARKER}:{}", crate::storage::source_key(source))
}
const DOMAIN: &str = "subagent_sessions::store";

fn collections() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new(SESSIONS), CollectionSpec::new(IMPORTED)]
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
                    .sort(Sort::asc("created_at"))
                    .sort(Sort::asc("_id")),
            )
            .await
        })?;
        stored
            .into_iter()
            .map(|item| Ok(serde_json::from_value(item.doc)?))
            .collect()
    }

    /// Whether the one-time legacy file import finished in this scope.
    pub(super) fn legacy_imported(&self, source: &std::path::Path) -> Result<bool> {
        let id = marker_id(source);
        let found = self
            .0
            .run(|docs| async move { docs.get(IMPORTED, &id).await })?;
        Ok(found.is_some())
    }

    /// Imports `sessions` read from the legacy file (a stored newer copy
    /// wins), then records the import in a marker document.
    pub(super) fn import_legacy(
        &self,
        source: &std::path::Path,
        sessions: &[DurableSubagentSession],
    ) -> Result<()> {
        let id = marker_id(source);
        let claimed =
            crate::storage::import_claim::with_claim(&self.0, IMPORTED, &id.clone(), || {
                // The process that held the claim before us may have finished.
                if self.legacy_imported(source)? {
                    return Ok(());
                }
                self.save(sessions)?;
                let id = id.clone();
                self.0.run(|docs| async move {
                    match docs
                        .put(
                            IMPORTED,
                            &id,
                            serde_json::json!({ "imported": true }),
                            Precondition::Absent,
                        )
                        .await
                    {
                        Ok(_) => Ok(()),
                        Err(error) if error.kind() == tinystoragedrivers::ErrorKind::Conflict => {
                            Ok(())
                        }
                        Err(error) => Err(error),
                    }
                })
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
            let mut doc = serde_json::to_value(session)?;
            doc["created_at"] = Value::from(session.created_at.as_str());
            let updated = chrono::DateTime::parse_from_rfc3339(&session.updated_at).ok();
            writes.push((session.subagent_session_id.clone(), doc, updated));
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
    updated: Option<chrono::DateTime<chrono::FixedOffset>>,
) -> Result<(), crate::storage::StorageError> {
    use tinystoragedrivers::ErrorKind;
    for _ in 0..crate::storage::documents::CAS_ATTEMPTS {
        let precondition = match docs.get(SESSIONS, id).await? {
            None => Precondition::Absent,
            Some(stored) => {
                let stored_at = stored
                    .doc
                    .get("updatedAt")
                    .and_then(Value::as_str)
                    .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok());
                if matches!((stored_at, updated), (Some(have), Some(ours)) if have >= ours) {
                    return Ok(());
                }
                stored.unchanged()
            }
        };
        match docs.put(SESSIONS, id, doc.clone(), precondition).await {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == ErrorKind::Conflict => {}
            Err(error) => return Err(error),
        }
    }
    Err(crate::storage::StorageError::conflict(format!(
        "{SESSIONS}/{id} kept changing"
    )))
}

#[cfg(test)]
#[path = "store_documents_tests.rs"]
mod tests;
