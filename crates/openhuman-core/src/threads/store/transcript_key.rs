//! The session-store key a thread's transcripts were written under.
//!
//! A thread's transcripts live in the session store of the agent key the
//! session host wrote them under (`agent::session_store::agent_key_for`:
//! `<profile>~<agent>` for a SaaS profile, else the agent, else the chat
//! agent). That key follows `[agent] chat_agent_id`, so after an operator
//! changes the chat agent, a reader that re-derives the key from the current
//! config looks in a different store and loses the thread's earlier turns.
//!
//! The key is therefore recorded with the thread the first time a session is
//! bound to it (first writer wins), and readers, edit and regenerate use the
//! recorded key. A thread with no record falls back to the current rule.

use anyhow::Result;
use serde_json::{json, Value};
use tinystoragedrivers::{CollectionSpec, ErrorKind, Filter, Precondition};

use super::backend::ConversationStore;
use super::storage_schema::{spec, TRANSCRIPT_KEYS as COLLECTION};
use crate::storage::documents::Repo;
use crate::storage::schema_support::key_of;
use crate::storage::{self};

const DOMAIN: &str = "threads::transcript_key";
const LOG_PREFIX: &str = "[conversations:transcript-key]";

fn collections() -> Vec<CollectionSpec> {
    vec![spec()]
}

impl ConversationStore {
    fn key_repo(&self) -> Result<Repo, String> {
        let scope = self.conversation_scope()?;
        let backend = storage::local::backend_for(&self.workspace_dir)
            .map_err(|e| format!("{LOG_PREFIX} open the storage backend: {e:#}"))?;
        Repo::on(&backend, &scope, DOMAIN, collections).map_err(|e| format!("{LOG_PREFIX} {e:#}"))
    }

    /// The key `thread_id`'s transcripts were first written under, if recorded.
    pub fn transcript_key(&self, thread_id: &str) -> Result<Option<String>, String> {
        let repo = self.key_repo()?;
        let id = key_of(thread_id).map_err(|e| format!("{LOG_PREFIX} {e}"))?;
        let doc = repo
            .run(|docs| async move { docs.get(COLLECTION, &id).await })
            .map_err(|e| format!("{LOG_PREFIX} read: {e:#}"))?;
        Ok(doc.and_then(|d| crate::storage::documents::text(&d.doc, "key").map(str::to_string)))
    }

    /// Records `key` as the thread's transcript key unless one is already
    /// recorded, and returns the key in effect (the first one recorded).
    pub fn record_transcript_key(&self, thread_id: &str, key: &str) -> Result<String, String> {
        let repo = self.key_repo()?;
        let id = key_of(thread_id).map_err(|e| format!("{LOG_PREFIX} {e}"))?;
        let wanted = key.to_string();
        let recorded = repo
            .run(|docs| async move {
                let body: Value = json!({ "key": wanted });
                match docs.put(COLLECTION, &id, body, Precondition::Absent).await {
                    Ok(_) => Ok(None),
                    Err(error) if error.kind() == ErrorKind::Conflict => {
                        let stored = docs.get(COLLECTION, &id).await?;
                        Ok(stored.and_then(|d| {
                            crate::storage::documents::text(&d.doc, "key").map(str::to_string)
                        }))
                    }
                    Err(error) => Err(error),
                }
            })
            .map_err(|e| format!("{LOG_PREFIX} record: {e:#}"))?;
        match recorded {
            Some(existing) => {
                log::debug!("{LOG_PREFIX} kept the recorded key for a thread");
                Ok(existing)
            }
            None => {
                log::debug!("{LOG_PREFIX} recorded the key for a thread");
                Ok(key.to_string())
            }
        }
    }

    /// Forgets `thread_id`'s recorded key (the thread was deleted).
    pub(super) fn forget_transcript_key(&self, thread_id: &str) {
        let outcome = self.key_repo().and_then(|repo| {
            let id = key_of(thread_id).map_err(|e| e.to_string())?;
            repo.run(|docs| async move { docs.delete(COLLECTION, &id, Precondition::None).await })
                .map(|_| ())
                .map_err(|e| format!("{e:#}"))
        });
        if let Err(error) = outcome {
            log::warn!("{LOG_PREFIX} forget failed: {error}");
        }
    }

    /// Forgets every recorded key (all threads were purged).
    pub(super) fn forget_all_transcript_keys(&self) {
        let outcome = self.key_repo().and_then(|repo| {
            repo.run(|docs| async move {
                docs.delete_where(COLLECTION, &Filter::exists("key", true))
                    .await
            })
            .map(|_| ())
            .map_err(|e| format!("{e:#}"))
        });
        if let Err(error) = outcome {
            log::warn!("{LOG_PREFIX} forget-all failed: {error}");
        }
    }
}

/// The key a session for `thread_id` must write its transcripts under: the
/// one already recorded with the thread, else `rule_key` (what the session host
/// would derive now), which is recorded as the thread's key. A store that
/// cannot be reached degrades to `rule_key` without recording.
pub(crate) async fn bind_transcript_key(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace_dir: &std::path::Path,
    thread_id: &str,
    rule_key: String,
) -> String {
    match super::blocking::record_transcript_key(
        ctx.clone(),
        workspace_dir.to_path_buf(),
        thread_id.to_string(),
        rule_key.clone(),
    )
    .await
    {
        Ok(key) => key,
        Err(error) => {
            log::warn!("{LOG_PREFIX} bind failed, using the current rule: {error}");
            rule_key
        }
    }
}

#[cfg(test)]
#[path = "transcript_key_tests.rs"]
mod tests;
