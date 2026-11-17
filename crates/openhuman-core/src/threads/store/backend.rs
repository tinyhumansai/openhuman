//! Chat threads on the storage backend: the workspace-keyed, synchronous
//! facade the host's callers use, over TinyAgents'
//! [`DriverConversationStore`] (the JSONL file store is gone upstream).
//!
//! Every call resolves the process's storage backend for the workspace
//! ([`crate::storage::local::backend_for`]: the installed backend, else the
//! workspace default) and binds it to the **conversation scope**: the calling
//! tenant's profile scope when there is one, else `local`. Conversations span
//! agents, so the acting agent's scope is deliberately not used: a thread a
//! sub-agent wrote must stay visible to the UI that lists it.
//!
//! The API is the one the file store had (`String` errors, owned arguments,
//! free functions beside [`ConversationStore`]) so the ~50 host callers did not
//! change. Each call blocks on the storage bridge ([`crate::storage::block_on`]);
//! request paths already go through [`super::blocking`], which keeps that off
//! the async workers.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};

use super::ConversationPurgeStats;
use tinyagents_session::threads::{
    ConversationError, ConversationPersistence, ConversationResult, ConversationThread,
    CreateConversationThread, CrossThreadHit, DriverConversationStore, ThreadMessage,
    ThreadMessagePatch,
};
use tinyagents_storage_kit::{Scope as AgentScope, ScopedStorage};

use crate::core::runtime::DispatchCtx;
use crate::storage::documents::Repo;
use crate::storage::{self, Scope, StorageError};
use tinystoragedrivers::ErrorKind;

#[derive(Default)]
struct ThreadAdmissionState {
    active_creates: usize,
    purging: bool,
}

struct WorkspaceThreadAdmission {
    state: Mutex<ThreadAdmissionState>,
    changed: Condvar,
}

static THREAD_ADMISSIONS: OnceLock<Mutex<HashMap<PathBuf, Weak<WorkspaceThreadAdmission>>>> =
    OnceLock::new();

fn normalized_workspace_path(workspace_dir: &Path) -> PathBuf {
    let absolute = if workspace_dir.is_absolute() {
        workspace_dir.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(workspace_dir)
    };
    if let Ok(canonical) = std::fs::canonicalize(&absolute) {
        return canonical;
    }
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    if let Ok(canonical) = std::fs::canonicalize(&normalized) {
        return canonical;
    }
    let mut existing = normalized.as_path();
    let mut suffix = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            break;
        };
        suffix.push(name.to_os_string());
        let Some(parent) = existing.parent() else {
            break;
        };
        existing = parent;
    }
    let mut key = std::fs::canonicalize(existing).unwrap_or_else(|_| existing.to_path_buf());
    for component in suffix.into_iter().rev() {
        key.push(component);
    }
    key
}

fn thread_admission(workspace_dir: &Path) -> Arc<WorkspaceThreadAdmission> {
    let admissions = THREAD_ADMISSIONS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut admissions = admissions.lock().unwrap_or_else(|error| error.into_inner());
    let workspace_key = normalized_workspace_path(workspace_dir);
    if let Some(admission) = admissions.get(&workspace_key).and_then(Weak::upgrade) {
        return admission;
    }
    admissions.retain(|_, admission| admission.strong_count() > 0);
    let admission = Arc::new(WorkspaceThreadAdmission {
        state: Mutex::new(ThreadAdmissionState::default()),
        changed: Condvar::new(),
    });
    admissions.insert(workspace_key, Arc::downgrade(&admission));
    admission
}

struct ThreadCreateGuard(Arc<WorkspaceThreadAdmission>);

impl Drop for ThreadCreateGuard {
    fn drop(&mut self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.active_creates -= 1;
        if state.active_creates == 0 {
            self.0.changed.notify_all();
        }
    }
}

impl WorkspaceThreadAdmission {
    fn admit_create(self: &Arc<Self>) -> Result<ThreadCreateGuard, String> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.purging {
            return Err("thread creation refused while workspace purge is active".into());
        }
        state.active_creates += 1;
        Ok(ThreadCreateGuard(Arc::clone(self)))
    }
}

/// A workspace-wide barrier that prevents new thread rows while a full purge
/// snapshots, cancels and removes the current rows.
pub(crate) struct ThreadPurgeGuard(Arc<WorkspaceThreadAdmission>);

impl Drop for ThreadPurgeGuard {
    fn drop(&mut self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.purging = false;
        self.0.changed.notify_all();
    }
}

/// Closes workspace thread admission and waits for in-flight creates to settle.
///
pub(crate) fn begin_thread_purge(workspace_dir: &Path) -> ThreadPurgeGuard {
    let admission = thread_admission(workspace_dir);
    let mut state = admission
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    while state.purging {
        state = admission
            .changed
            .wait(state)
            .unwrap_or_else(|error| error.into_inner());
    }
    state.purging = true;
    while state.active_creates > 0 {
        state = admission
            .changed
            .wait(state)
            .unwrap_or_else(|error| error.into_inner());
    }
    drop(state);
    ThreadPurgeGuard(admission)
}

const DOMAIN: &str = "threads::store";
const LOG_PREFIX: &str = "[conversations]";

/// The storage scope conversations live under: the calling tenant's profile,
/// else `local`; refused in SaaS mode with no profile.
fn conversation_scope(ctx: &DispatchCtx) -> Result<Scope, String> {
    let tenant = ctx.tenant().map_err(|e| e.to_string())?;
    storage::scope_from(
        tenant.profile.as_deref(),
        None,
        crate::core::runtime::mode::is_saas(),
    )
    .map_err(|e| e.to_string())
}

/// What an operation addresses, to word a not-found error as the file store did.
#[derive(Clone, Copy)]
struct Subject<'a> {
    thread: Option<&'a str>,
    message: Option<&'a str>,
}

const NONE: Subject<'static> = Subject {
    thread: None,
    message: None,
};

const fn thread(id: &str) -> Subject<'_> {
    Subject {
        thread: Some(id),
        message: None,
    }
}

const fn message<'a>(thread: &'a str, message: &'a str) -> Subject<'a> {
    Subject {
        thread: Some(thread),
        message: Some(message),
    }
}

/// The error text callers match on: a missing thread or message reads
/// `thread <id> not found` / `message <id> not found in thread <id>` (the
/// wording the file store used and `threads::ops` maps to a typed error);
/// anything else is the driver's own message.
fn describe(error: &ConversationError, subject: Subject<'_>) -> String {
    if let ConversationError::Driver(inner) = error {
        if inner.kind() == ErrorKind::NotFound {
            let text = inner.to_string();
            match (subject.thread, subject.message) {
                (Some(t), Some(m)) if text.contains("message") => {
                    return format!("message {m} not found in thread {t}");
                }
                (Some(t), _) => return format!("thread {t} not found"),
                _ => {}
            }
        }
    }
    error.to_string()
}

/// Imports the legacy JSONL conversations under `workspace_dir` into the
/// `local` conversation scope of an explicit `backend`, once (the same
/// `tsd.migrations` entry [`ConversationStore`] checks). For the layout
/// migration (`storage::layout`), which runs before any store opens the new
/// database; the source files are only read.
///
/// # Errors
///
/// A storage failure or an unreadable legacy file; the next call retries.
pub(crate) fn import_legacy_into(
    backend: &Arc<dyn storage::StorageBackend>,
    workspace_dir: &Path,
) -> Result<(), String> {
    let scope = Scope::local();
    let scoped = backend
        .for_scope(&scope)
        .map_err(|e| format!("{LOG_PREFIX} open the storage scope: {e}"))?;
    let repo = Repo::on(backend, &scope, DOMAIN, super::legacy_import::collections)
        .map_err(|e| format!("{LOG_PREFIX} {e:#}"))?;
    let _store = DriverConversationStore::new(ScopedStorage::new(
        scoped.clone(),
        AgentScope::new(scope.as_str()).map_err(|e| format!("{LOG_PREFIX} invalid scope: {e}"))?,
    ));
    let docs = Arc::clone(scoped.documents());
    storage::block_on(async move {
        DriverConversationStore::prepare_schema(docs.as_ref()).await?;
        DriverConversationStore::prepare_append_schema(docs.as_ref()).await
    })
    .map_err(|e| format!("{LOG_PREFIX} declare the conversation schema: {e}"))?;
    super::legacy_import::import_once(workspace_dir, &repo)
        .map_err(|e| format!("{LOG_PREFIX} legacy import: {e:#}"))
}

/// A conversation store bound to one workspace's storage backend.
#[derive(Clone)]
pub struct ConversationStore {
    pub(super) ctx: DispatchCtx,
    pub(super) workspace_dir: PathBuf,
}

impl ConversationStore {
    /// A store for `workspace_dir`, acting for the tenant `ctx` serves.
    /// Nothing is opened until the first call.
    pub fn new(ctx: DispatchCtx, workspace_dir: PathBuf) -> Self {
        Self { ctx, workspace_dir }
    }

    pub(super) fn conversation_scope(&self) -> Result<Scope, String> {
        conversation_scope(&self.ctx)
    }

    /// The driver store for this call, its schema declared once per backend
    /// and scope, and the legacy JSONL conversations imported once.
    fn open(&self) -> Result<DriverConversationStore, String> {
        let scope = conversation_scope(&self.ctx)?;
        let backend = storage::local::backend_for(&self.workspace_dir)
            .map_err(|e| format!("{LOG_PREFIX} open the storage backend: {e:#}"))?;
        let scoped = backend
            .for_scope(&scope)
            .map_err(|e| format!("{LOG_PREFIX} open the storage scope: {e}"))?;
        let repo = Repo::on(&backend, &scope, DOMAIN, super::legacy_import::collections)
            .map_err(|e| format!("{LOG_PREFIX} {e:#}"))?;
        let owner_storage = ScopedStorage::new(
            scoped.clone(),
            AgentScope::new(scope.as_str())
                .map_err(|e| format!("{LOG_PREFIX} invalid scope: {e}"))?,
        );
        let store = DriverConversationStore::new(owner_storage.clone());
        repo.once_per_backend("schema", || {
            let leased_storage = owner_storage.clone();
            storage::block_on(async move {
                let migration_storage = leased_storage.clone();
                tinyagents_storage_kit::with_migration_lease(
                    &leased_storage,
                    "conversations",
                    move |_| async move {
                        let docs = Arc::clone(migration_storage.documents());
                        DriverConversationStore::prepare_schema(docs.as_ref()).await?;
                        DriverConversationStore::prepare_append_schema(docs.as_ref()).await?;
                        for spec in tinyagents_storage_kit::primitives::collection_specs() {
                            docs.ensure_collection(&spec).await?;
                        }
                        let registry = tinyagents_storage_kit::Registry::compose([
                            tinyagents_session::threads::conversations_storage_schema(),
                        ])?;
                        registry
                            .migrate(
                                &migration_storage,
                                &tinyagents_storage_kit::MigrateOptions::new(),
                            )
                            .await?;
                        Ok(())
                    },
                )
                .await
            })
            .map_err(|e| anyhow::anyhow!("prepare the conversation schema: {e}"))
        })
        .map_err(|e| format!("{LOG_PREFIX} {e:#}"))?;
        super::legacy_import::import_once(&self.workspace_dir, &repo)
            .map_err(|e| format!("{LOG_PREFIX} legacy import: {e:#}"))?;
        Ok(store)
    }

    fn run<T, F, Fut>(
        &self,
        operation: &'static str,
        subject: Subject<'_>,
        f: F,
    ) -> Result<T, String>
    where
        F: FnOnce(DriverConversationStore) -> Fut,
        Fut: std::future::Future<Output = ConversationResult<T>> + Send + 'static,
        T: Send + 'static,
    {
        let store = self.open()?;
        let future = f(store);
        let outcome = storage::block_on(async move { Ok(future.await) })
            .map_err(|e: StorageError| format!("{LOG_PREFIX} {operation}: {e}"))?;
        outcome.map_err(|e| {
            log::debug!("{LOG_PREFIX} {operation} failed: {e}");
            describe(&e, subject)
        })
    }

    /// Creates the thread or returns the existing one.
    pub fn ensure_thread(
        &self,
        request: CreateConversationThread,
    ) -> Result<ConversationThread, String> {
        let _admission = thread_admission(&self.workspace_dir).admit_create()?;
        self.run("ensure_thread", NONE, |s| async move {
            s.ensure_thread(request).await
        })
    }

    /// Every live thread, newest first.
    pub fn list_threads(&self) -> Result<Vec<ConversationThread>, String> {
        self.run(
            "list_threads",
            NONE,
            |s| async move { s.list_threads().await },
        )
    }

    /// A thread's messages in append order; empty for an unknown thread.
    pub fn get_messages(&self, thread_id: &str) -> Result<Vec<ThreadMessage>, String> {
        let id = thread_id.to_string();
        self.run("get_messages", thread(thread_id), |s| async move {
            if s.get_thread(&id).await?.is_none() {
                return Ok(Vec::new());
            }
            s.get_messages(&id).await
        })
    }

    /// Cross-thread substring search over message content.
    pub fn search_cross_thread_messages(
        &self,
        query: &str,
        limit: usize,
        exclude_thread_id: Option<&str>,
    ) -> Result<Vec<CrossThreadHit>, String> {
        let query = query.to_string();
        let exclude = exclude_thread_id.map(str::to_string);
        self.run("search_cross_thread_messages", NONE, |s| async move {
            s.search_cross_thread_messages(&query, limit, exclude.as_deref())
                .await
        })
    }

    /// Appends a message to a thread.
    pub fn append_message(
        &self,
        thread_id: &str,
        message: ThreadMessage,
    ) -> Result<ThreadMessage, String> {
        let id = thread_id.to_string();
        self.run("append_message", thread(thread_id), |s| async move {
            s.append_message(&id, message).await
        })
    }

    /// Renames a thread.
    pub fn update_thread_title(
        &self,
        thread_id: &str,
        title: &str,
        updated_at: &str,
    ) -> Result<ConversationThread, String> {
        let (id, title, at) = (
            thread_id.to_string(),
            title.to_string(),
            updated_at.to_string(),
        );
        self.run("update_thread_title", thread(thread_id), |s| async move {
            s.update_thread_title(&id, &title, &at).await
        })
    }

    /// Sets a thread's working directory.
    pub fn update_thread_working_dir(
        &self,
        thread_id: &str,
        working_dir: Option<String>,
        updated_at: &str,
    ) -> Result<ConversationThread, String> {
        let (id, at) = (thread_id.to_string(), updated_at.to_string());
        self.run(
            "update_thread_working_dir",
            thread(thread_id),
            |s| async move { s.update_thread_working_dir(&id, working_dir, &at).await },
        )
    }

    /// Replaces a thread's labels.
    pub fn update_thread_labels(
        &self,
        thread_id: &str,
        labels: Vec<String>,
        updated_at: &str,
    ) -> Result<ConversationThread, String> {
        let (id, at) = (thread_id.to_string(), updated_at.to_string());
        self.run("update_thread_labels", thread(thread_id), |s| async move {
            s.update_thread_labels(&id, labels, &at).await
        })
    }

    /// Patches one message.
    pub fn update_message(
        &self,
        thread_id: &str,
        message_id: &str,
        patch: ThreadMessagePatch,
    ) -> Result<ThreadMessage, String> {
        let (id, mid) = (thread_id.to_string(), message_id.to_string());
        self.run(
            "update_message",
            message(thread_id, message_id),
            |s| async move { s.update_message(&id, &mid, patch).await },
        )
    }

    /// Removes a message and everything after it; `None` for an unknown id.
    pub fn delete_messages_from(
        &self,
        thread_id: &str,
        message_id: &str,
    ) -> Result<Option<usize>, String> {
        let (id, mid) = (thread_id.to_string(), message_id.to_string());
        self.run(
            "delete_messages_from",
            message(thread_id, message_id),
            |s| async move { s.delete_messages_from(&id, &mid).await },
        )
    }

    /// Soft-deletes a thread; whether it existed.
    pub fn delete_thread(&self, thread_id: &str, deleted_at: &str) -> Result<bool, String> {
        let (id, at) = (thread_id.to_string(), deleted_at.to_string());
        let deleted = self.run("delete_thread", thread(thread_id), |s| async move {
            s.delete_thread(&id, &at).await
        })?;
        self.forget_transcript_key(thread_id);
        Ok(deleted)
    }

    /// Removes every thread and records its conversation-memory deletion in
    /// this workspace's existing best-effort queue as each header is removed.
    /// Queue persistence keeps its existing IO and process-crash limitations.
    pub fn purge_threads(&self) -> Result<ConversationPurgeStats, String> {
        let workspace_dir = self.workspace_dir.clone();
        let deletion_state = crate::memory::deletion::open_state(&workspace_dir)
            .map_err(|error| format!("{LOG_PREFIX} open deletion queue: {error:#}"))?;
        let stats = self.run("purge_threads", NONE, |s| async move {
            let callback_state = deletion_state;
            s.purge_threads_with_async(move |thread_id| {
                let state = callback_state.clone();
                async move {
                    if let Err(error) = crate::memory::deletion::enqueue_with_state(
                        &state,
                        crate::memory::deletion::PendingDeletion::Thread { thread_id },
                    )
                    .await
                    {
                        log::warn!(
                            "[threads] queue conversation memory deletion failed: {error:#}"
                        );
                    }
                }
            })
            .await
            .map(ConversationPurgeStats::from)
        })?;
        self.forget_all_transcript_keys();
        Ok(stats)
    }
}

fn store(ctx: &DispatchCtx, workspace_dir: PathBuf) -> ConversationStore {
    ConversationStore::new(ctx.clone(), workspace_dir)
}

/// Free-function shim around [`ConversationStore::ensure_thread`].
pub fn ensure_thread(
    ctx: &DispatchCtx,
    workspace_dir: PathBuf,
    request: CreateConversationThread,
) -> Result<ConversationThread, String> {
    store(ctx, workspace_dir).ensure_thread(request)
}

/// Free-function shim around [`ConversationStore::list_threads`].
pub fn list_threads(
    ctx: &DispatchCtx,
    workspace_dir: PathBuf,
) -> Result<Vec<ConversationThread>, String> {
    store(ctx, workspace_dir).list_threads()
}

/// Free-function shim around [`ConversationStore::get_messages`].
pub fn get_messages(
    ctx: &DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: &str,
) -> Result<Vec<ThreadMessage>, String> {
    store(ctx, workspace_dir).get_messages(thread_id)
}

/// Free-function shim around [`ConversationStore::append_message`].
pub fn append_message(
    ctx: &DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: &str,
    message: ThreadMessage,
) -> Result<ThreadMessage, String> {
    store(ctx, workspace_dir).append_message(thread_id, message)
}

/// Free-function shim around [`ConversationStore::update_thread_title`].
pub fn update_thread_title(
    ctx: &DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: &str,
    title: &str,
    updated_at: &str,
) -> Result<ConversationThread, String> {
    store(ctx, workspace_dir).update_thread_title(thread_id, title, updated_at)
}

/// Free-function shim around [`ConversationStore::update_thread_labels`].
pub fn update_thread_labels(
    ctx: &DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: &str,
    labels: Vec<String>,
    updated_at: &str,
) -> Result<ConversationThread, String> {
    store(ctx, workspace_dir).update_thread_labels(thread_id, labels, updated_at)
}

/// Free-function shim around [`ConversationStore::update_thread_working_dir`].
pub fn update_thread_working_dir(
    ctx: &DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: &str,
    working_dir: Option<String>,
    updated_at: &str,
) -> Result<ConversationThread, String> {
    store(ctx, workspace_dir).update_thread_working_dir(thread_id, working_dir, updated_at)
}

/// Free-function shim around [`ConversationStore::update_message`].
pub fn update_message(
    ctx: &DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: &str,
    message_id: &str,
    patch: ThreadMessagePatch,
) -> Result<ThreadMessage, String> {
    store(ctx, workspace_dir).update_message(thread_id, message_id, patch)
}

/// Free-function shim around [`ConversationStore::delete_messages_from`].
pub fn delete_messages_from(
    ctx: &DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: &str,
    message_id: &str,
) -> Result<Option<usize>, String> {
    store(ctx, workspace_dir).delete_messages_from(thread_id, message_id)
}

/// Free-function shim around [`ConversationStore::purge_threads`].
pub fn purge_threads(
    ctx: &DispatchCtx,
    workspace_dir: PathBuf,
) -> Result<ConversationPurgeStats, String> {
    store(ctx, workspace_dir).purge_threads()
}

/// Free-function shim around [`ConversationStore::delete_thread`].
pub fn delete_thread(
    ctx: &DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: &str,
    deleted_at: &str,
) -> Result<bool, String> {
    store(ctx, workspace_dir).delete_thread(thread_id, deleted_at)
}

/// Whether `path` is the legacy conversation root of `workspace_dir`.
pub(super) fn legacy_root(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join("memory").join("conversations")
}
