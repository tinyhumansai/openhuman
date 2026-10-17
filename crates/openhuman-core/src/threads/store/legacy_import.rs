//! One-time, read-only import of the retired JSONL conversation files.
//!
//! Older installs kept chat threads as `threads.jsonl` (an append-only
//! upsert/delete log) plus one `threads/<hex(thread_id)>.jsonl` per thread
//! under `<workspace>/memory/conversations/`. The file store is gone, so the
//! first use of a workspace's backend folds those files into it. The files are
//! only ever **read**: a `tsd.migrations` entry in the backend records
//! completion, and the legacy tree stays byte-identical (`storage::import` follows the same
//! rule for the per-domain databases).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tinyagents_session::threads::{
    ConversationPersistence, CreateConversationThread, DriverConversationStore, ThreadMessage,
};
use tinystoragedrivers::CollectionSpec;

use crate::storage::documents::Repo;
use crate::storage::import_state;

/// The `tsd.migrations` owner of the import.
const OWNER: &str = "threads";
/// The old marker collection, now the `kind` of the import's step.
const IMPORT_KIND: &str = "threads_import";
const MARKER_PREFIX: &str = "legacy-conversations";
const LOG_PREFIX: &str = "[conversations:import]";

/// The collections the thread host declares: the transcript keys. The import
/// state is in `tsd.migrations`, which `Registry::prepare` declares.
pub(super) fn collections() -> Vec<CollectionSpec> {
    vec![super::storage_schema::spec()]
}

/// The `threads.jsonl` records the importer needs; the stats records the old
/// store derived (`message_appended`, `stats`) are recomputed from messages.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum LogEntry {
    Upsert {
        thread_id: String,
        title: String,
        created_at: String,
        #[serde(default)]
        parent_thread_id: Option<String>,
        #[serde(default)]
        labels: Option<Vec<String>>,
        #[serde(default)]
        personality_id: Option<String>,
        #[serde(default)]
        working_dir: Option<String>,
    },
    Delete {
        thread_id: String,
    },
    #[serde(other)]
    Derived,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `threads/<stem>.jsonl` for a thread: hex of short ids, a SHA-256 digest for
/// longer ones, as the file store named them.
fn messages_path(root: &Path, thread_id: &str) -> std::path::PathBuf {
    let stem = if thread_id.len() <= 124 {
        hex(thread_id.as_bytes())
    } else {
        format!("sha256-{}", hex(&Sha256::digest(thread_id.as_bytes())))
    };
    root.join("threads").join(format!("{stem}.jsonl"))
}

/// Parseable lines of a JSONL file; a torn or foreign line is skipped, as the
/// file store skipped it. A missing file is empty.
fn read_jsonl<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Vec<T>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(anyhow!("read {}: {error}", path.display())),
    };
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str::<T>(line.trim()).ok())
        .collect())
}

/// Folds the upsert/delete log into the live threads, in first-seen order.
fn fold_threads(root: &Path) -> Result<Vec<CreateConversationThread>> {
    let mut live: BTreeMap<String, CreateConversationThread> = BTreeMap::new();
    for entry in read_jsonl::<LogEntry>(&root.join("threads.jsonl"))? {
        match entry {
            LogEntry::Upsert {
                thread_id,
                title,
                created_at,
                parent_thread_id,
                labels,
                personality_id,
                working_dir,
            } => match live.get_mut(&thread_id) {
                Some(existing) => {
                    existing.title = title;
                    existing.parent_thread_id =
                        parent_thread_id.or(existing.parent_thread_id.take());
                    existing.labels = labels.or(existing.labels.take());
                    existing.personality_id = personality_id.or(existing.personality_id.take());
                    existing.working_dir = match working_dir {
                        Some(dir) if dir.is_empty() => None,
                        Some(dir) => Some(dir),
                        None => existing.working_dir.take(),
                    };
                }
                None => {
                    live.insert(
                        thread_id.clone(),
                        CreateConversationThread {
                            id: thread_id,
                            title,
                            created_at,
                            parent_thread_id,
                            labels,
                            personality_id,
                            working_dir: working_dir.filter(|dir| !dir.is_empty()),
                        },
                    );
                }
            },
            LogEntry::Delete { thread_id } => {
                live.remove(&thread_id);
            }
            LogEntry::Derived => {}
        }
    }
    Ok(live.into_values().collect())
}

/// Imports the workspace's legacy conversations into `store` once per backend
/// and scope. `repo` carries the marker collection ([`collections`]).
///
/// # Errors
///
/// A legacy file that cannot be read, or a storage failure; the next call
/// retries (a thread or message already imported is skipped).
pub(super) fn import_once(
    workspace_dir: &Path,
    repo: &Repo,
    store: &DriverConversationStore,
) -> Result<()> {
    let root = super::backend::legacy_root(workspace_dir);
    let source = crate::storage::source_key(&root);
    let step = import_state::step_id(IMPORT_KIND, &format!("{MARKER_PREFIX}:{source}"));
    repo.once_per_backend(&format!("legacy-import:{source}"), || {
        match std::fs::metadata(root.join("threads.jsonl")) {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(anyhow::Error::new(error).context("stat the legacy threads log"))
            }
        }
        if import_state::is_done(repo, OWNER, &step)? {
            log::debug!("{LOG_PREFIX} already imported");
            return Ok(());
        }
        let claimed = import_state::with_claim(repo, OWNER, &step, || {
            if import_state::is_done(repo, OWNER, &step)? {
                return Ok(());
            }
            let threads = fold_threads(&root)?;
            let mut messages = Vec::with_capacity(threads.len());
            for thread in &threads {
                messages.push(read_jsonl::<ThreadMessage>(&messages_path(
                    &root, &thread.id,
                ))?);
            }
            let store = store.clone();
            let (threads_n, messages_n) =
                crate::storage::block_on_anyhow(
                    async move { copy(&store, threads, messages).await },
                )?;
            import_state::set_done(repo, OWNER, &step)?;
            log::info!("{LOG_PREFIX} imported threads={threads_n} messages={messages_n}");
            Ok(())
        })?;
        if claimed.is_none() {
            log::debug!("{LOG_PREFIX} another process is importing; skipped");
        }
        Ok(())
    })
}

/// Writes the folded threads and their messages; one already present (an
/// earlier interrupted import) is not duplicated.
async fn copy(
    store: &DriverConversationStore,
    threads: Vec<CreateConversationThread>,
    messages: Vec<Vec<ThreadMessage>>,
) -> Result<(usize, usize)> {
    let mut message_total = 0;
    for (thread, thread_messages) in threads.into_iter().zip(messages) {
        let id = thread.id.clone();
        store
            .ensure_thread(thread)
            .await
            .with_context(|| format!("create thread {id}"))?;
        let present: std::collections::HashSet<String> = store
            .get_messages(&id)
            .await
            .with_context(|| format!("read thread {id}"))?
            .into_iter()
            .map(|m| m.id)
            .collect();
        for message in thread_messages {
            if present.contains(&message.id) {
                continue;
            }
            store
                .append_message(&id, message)
                .await
                .with_context(|| format!("append to thread {id}"))?;
            message_total += 1;
        }
    }
    let thread_total = store.list_threads().await?.len();
    Ok((thread_total, message_total))
}
