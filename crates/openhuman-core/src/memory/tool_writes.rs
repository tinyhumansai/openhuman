//! Durable, bounded FIFO for the agent tool's optional memory writes.
//!
//! The queue itself (bounds, coalescing, fencing, ordered drain) is
//! `tinymemory_tools::pending`; this module is the host adapter. It keeps the
//! per-owner partitioning of the files under `<workspace>/memory/`, the
//! validation and scrubbing of a write before it is queued, the file-backed
//! `ListStore` (owner-only, fsynced), the executor that runs a write against
//! the bound engine under a timeout, and the erasure barrier that waits for a
//! write already in flight. Only scrubbed items and trusted scope are
//! persisted; credentials are resolved at drain time. Enqueue never waits on
//! the remote engine.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tinymemory_api::{Namespace, StoreItem, WriteOptions};
use tinymemory_tools::list_store::{ListStore, StoreError};
use tinymemory_tools::pending::{
    Ack, Binding, Entry, Epoch, FenceToken, Limits, PendingError, PendingWrites, WriteExecutor,
    WriteGate, WriteOperation,
};

use super::error::{MemoryError, MemoryResult};
use super::tools::CallFacts;
use super::types::{ForgetParams, LearnParams};
use crate::config::Config;

const MAX_PENDING: usize = 256;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const WRITE_TIMEOUT: Duration = Duration::from_secs(120);
static PROCESS_EPOCH: LazyLock<String> = LazyLock::new(|| uuid::Uuid::new_v4().to_string());
const QUEUED: &str = "queued; not yet saved to memory";
static LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Default)]
struct DrainState {
    barrier: Arc<tokio::sync::Mutex<()>>,
}

/// One owner's erasure state: the module's gate (generation, fences) and the
/// barrier an erasure takes exclusively to wait for the write in flight.
/// Kept for the life of the process (a few bytes per owner) because a fence
/// token borrows its gate.
struct OwnerState {
    gate: WriteGate,
    barrier: Arc<tokio::sync::RwLock<()>>,
}

type Owners = HashMap<PathBuf, &'static OwnerState>;
static OWNERS: LazyLock<Mutex<Owners>> = LazyLock::new(|| Mutex::new(HashMap::new()));

type DrainLocks = HashMap<PathBuf, std::sync::Weak<DrainState>>;
static DRAINS: LazyLock<Mutex<DrainLocks>> = LazyLock::new(|| Mutex::new(HashMap::new()));

type Queue = PendingWrites<FileStore>;
static QUEUES: LazyLock<Mutex<HashMap<PathBuf, Arc<Queue>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// An entry as older builds wrote it: the owner and layout as two fields.
#[derive(Deserialize)]
struct LegacyEntry {
    ticket: String,
    root: Option<String>,
    layout_v3: bool,
    scope_root: Namespace,
    #[serde(default)]
    epoch: Option<(String, u64)>,
    operation: WriteOperation,
    status: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Stored {
    Current(Entry),
    Legacy(LegacyEntry),
}

impl From<Stored> for Entry {
    fn from(stored: Stored) -> Self {
        match stored {
            Stored::Current(entry) => entry,
            Stored::Legacy(old) => Entry {
                ticket: old.ticket,
                binding: binding_of(old.root, old.layout_v3),
                scope_root: old.scope_root,
                epoch: old.epoch.map(|(process, generation)| Epoch {
                    process,
                    generation,
                }),
                operation: old.operation,
                status: old.status,
            },
        }
    }
}

fn binding_of(root: Option<String>, layout_v3: bool) -> Binding {
    Binding(serde_json::to_string(&(root, layout_v3)).expect("binding fields serialize"))
}

fn binding(config: &Config) -> Binding {
    binding_of(
        super::scope::user_root(config),
        super::scope::layout_is_v3(config),
    )
}

#[cfg(test)]
fn path(workspace: &Path) -> PathBuf {
    let config = Config {
        workspace_dir: workspace.to_path_buf(),
        config_path: workspace
            .parent()
            .expect("fixture root")
            .join("config.toml"),
        ..Config::default()
    };
    queue_path(&config, &Namespace::ROOT)
}

fn owner_prefix(config: &Config) -> String {
    let root = super::scope::user_root(config);
    let fallback = root
        .is_none()
        .then(|| config.config_path.to_string_lossy().to_string());
    let owner = serde_json::to_vec(&(root, fallback)).expect("owner fields serialize");
    format!("tool_writes-{:x}", Sha256::digest(owner))
}

fn queue_path(config: &Config, root: &Namespace) -> PathBuf {
    let partition = serde_json::to_vec(&(
        &config.memory.root,
        super::scope::layout_is_v3(config),
        root,
    ))
    .expect("partition fields serialize");
    config.workspace_dir.join("memory").join(format!(
        "{}-{:x}.json",
        owner_prefix(config),
        Sha256::digest(partition)
    ))
}

fn unavailable() -> MemoryError {
    MemoryError::Unavailable(
        "memory write queue unavailable; durable queueing was not confirmed".into(),
    )
}

/// Accepts a validated, scrubbed write locally. The acknowledgement means only
/// that it is durable in the local outbox, not accepted by the remote engine.
pub(super) fn enqueue(
    config: &Config,
    args: &serde_json::Value,
    facts: &CallFacts,
) -> MemoryResult<serde_json::Value> {
    if super::engine::switched_off(config) {
        return Err(MemoryError::Off("memory is disabled".into()));
    }
    let confinement = super::user_scope::confinement(config)?;
    let reach = match confinement {
        Some(ref root) => super::user_scope::clamp_reach(Some(facts.reach.clone()), root),
        None => facts.reach.clone(),
    };
    let scope_root = reach.at.clone();
    let file = queue_path(config, &scope_root);
    let operation = match args.get("action").and_then(serde_json::Value::as_str) {
        Some("learn") => {
            let mut params: LearnParams = serde_json::from_value(args.clone())
                .map_err(|_| MemoryError::invalid("invalid learning arguments"))?;
            params.meta = None;
            let mut item = super::ops::learning_item(params, Some(facts.learn_meta()))?;
            if let Some(root) = confinement {
                super::user_scope::clamp_item(&mut item, &root);
            }
            if !super::user_scope::within(&item.meta().namespace, &reach.at) || reach.inherit {
                return Err(MemoryError::invalid(
                    "learning namespace is outside the trusted reach",
                ));
            }
            WriteOperation::Learn {
                item: super::guard::scrub(item),
            }
        }
        Some("forget") => {
            let params: ForgetParams = serde_json::from_value(args.clone())
                .map_err(|_| MemoryError::invalid("invalid forgetting arguments"))?;
            let mut ids: Vec<_> = params
                .ids
                .into_iter()
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .collect();
            ids.sort();
            ids.dedup();
            if ids.is_empty() {
                return Err(MemoryError::invalid("forget needs at least one id"));
            }
            WriteOperation::Forget { ids, reach }
        }
        _ => return Err(MemoryError::invalid("expected a memory write")),
    };
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !file.exists() && partitions(config)?.len() >= MAX_PENDING {
        return Err(MemoryError::Unavailable(
            "memory write queue full; nothing was queued".into(),
        ));
    }
    let owner = owner_state(config);
    let ack = queue(&file)
        .enqueue(operation, scope_root, binding(config), &owner.gate)
        .map_err(map_pending)?;
    Ok(match ack {
        Ack::Learned { id } => serde_json::json!({"id":id, "status":QUEUED}),
        Ack::Forgotten { queued } => {
            serde_json::json!({"queued":queued, "status":"queued; not yet removed from memory"})
        }
    })
}

/// A [`ListStore`] over one queue file: owner-only, written through a
/// temporary file and a rename, with the file and its directory fsynced.
struct FileStore {
    file: PathBuf,
}

impl ListStore<Entry> for FileStore {
    fn load(&self) -> Result<Vec<Entry>, StoreError> {
        read_file(&self.file).map_err(|_| StoreError("queue file unreadable".into()))
    }

    fn save(&self, entries: &[Entry]) -> Result<(), StoreError> {
        write_file(&self.file, entries).map_err(|_| StoreError("queue file unwritable".into()))
    }
}

fn read_file(path: &Path) -> std::io::Result<Vec<Entry>> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Err(std::io::Error::other("queue file too large"));
    }
    let stored: Vec<Stored> = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
    Ok(stored.into_iter().map(Entry::from).collect())
}

fn write_file(file: &Path, entries: &[Entry]) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(entries).map_err(std::io::Error::other)?;
    let temp = file.with_extension("json.tmp");
    super::files::create_private_dir_all(file.parent().expect("memory directory"))?;
    let mut output = super::files::create_private(&temp)?;
    output.write_all(&bytes)?;
    output.sync_all()?;
    std::fs::rename(&temp, file)?;
    #[cfg(unix)]
    std::fs::File::open(file.parent().expect("memory directory"))?.sync_all()?;
    Ok(())
}

fn queue(file: &Path) -> Arc<Queue> {
    let mut queues = QUEUES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    queues
        .entry(file.to_path_buf())
        .or_insert_with(|| {
            Arc::new(PendingWrites::new(
                FileStore {
                    file: file.to_path_buf(),
                },
                Limits {
                    max_entries: MAX_PENDING,
                    max_bytes: MAX_BYTES,
                },
            ))
        })
        .clone()
}

fn map_pending(error: PendingError) -> MemoryError {
    MemoryError::Unavailable(error.to_string())
}

fn read_at(path: &Path) -> MemoryResult<Vec<Entry>> {
    queue(path).entries().map_err(map_pending)
}

#[cfg(test)]
fn write_at(file: &Path, entries: &[Entry]) -> MemoryResult<()> {
    FileStore {
        file: file.to_path_buf(),
    }
    .save(entries)
    .map_err(|_| unavailable())
}

#[cfg(test)]
fn read(workspace: &Path) -> MemoryResult<Vec<Entry>> {
    read_at(&path(workspace))
}
#[cfg(test)]
fn write(workspace: &Path, entries: &[Entry]) -> MemoryResult<()> {
    write_at(&path(workspace), entries)
}

fn drain_state(workspace: &Path) -> Arc<DrainState> {
    let mut locks = DRAINS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    locks.retain(|_, value| value.strong_count() > 0);
    if let Some(lock) = locks.get(workspace).and_then(std::sync::Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(DrainState::default());
    locks.insert(workspace.to_path_buf(), Arc::downgrade(&lock));
    lock
}

fn owner_state(config: &Config) -> &'static OwnerState {
    let key = config
        .workspace_dir
        .join("memory")
        .join(owner_prefix(config));
    let mut owners = OWNERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    owners.entry(key).or_insert_with(|| {
        Box::leak(Box::new(OwnerState {
            gate: WriteGate::new(PROCESS_EPOCH.clone()),
            barrier: Arc::default(),
        }))
    })
}

fn partitions(config: &Config) -> MemoryResult<Vec<PathBuf>> {
    let prefix = format!("{}-", owner_prefix(config));
    let entries = match std::fs::read_dir(config.workspace_dir.join("memory")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(unavailable()),
    };
    let mut files = Vec::new();
    for entry in entries {
        let file = entry.map_err(|_| unavailable())?.path();
        if file
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".json"))
        {
            files.push(file);
            if files.len() > MAX_PENDING {
                return Err(unavailable());
            }
        }
    }
    Ok(files)
}

/// Owner epoch captured when automatic turn work is admitted, before it can
/// wait in the prefetch queue. Erasure retires older work and waits for active
/// mutations while subsequent work waits behind the exclusive barrier.
pub(crate) struct AutomaticMutation {
    state: &'static OwnerState,
    generation: u64,
}

impl AutomaticMutation {
    pub(crate) async fn run<T>(self, work: impl std::future::Future<Output = T>) -> Option<T> {
        if self.state.gate.generation() != self.generation {
            return None;
        }
        let _barrier = self.state.barrier.read().await;
        if self.state.gate.generation() != self.generation {
            return None;
        }
        Some(work.await)
    }
}

/// Captures authority without waiting on the engine or an active erasure.
pub(crate) fn automatic_mutation(config: &Config) -> AutomaticMutation {
    let state = owner_state(config);
    AutomaticMutation {
        state,
        generation: state.gate.generation(),
    }
}

/// Holds the owner's mutation barrier through explicit engine erasure.
/// Subsequent queued writes resume only after the outer erasure finishes.
pub(crate) struct WriteFence {
    guard: Option<tokio::sync::OwnedRwLockWriteGuard<()>>,
    token: Option<FenceToken<'static>>,
    state: &'static OwnerState,
    config: Arc<Config>,
    ctx: crate::core::runtime::DispatchCtx,
}

impl Drop for WriteFence {
    fn drop(&mut self) {
        drop(self.token.take());
        drop(self.guard.take());
        if !self.state.gate.is_fenced() && tokio::runtime::Handle::try_current().is_ok() {
            schedule_all(&self.ctx, self.config.clone());
        }
    }
}

/// Stops each owner partition after its current mutation, durably clears
/// earlier pending writes, and fences subsequent writes through explicit erase.
/// Other owners sharing a workspace keep independent queues and barriers.
pub(crate) async fn fence_and_clear(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
) -> MemoryResult<WriteFence> {
    let state = owner_state(config);
    let token = state.gate.begin_fence();
    let fencing_generation = token.generation();
    let mut fence = WriteFence {
        guard: None,
        token: Some(token),
        state,
        config: Arc::new(config.clone()),
        ctx: ctx.clone(),
    };
    fence.guard = Some(state.barrier.clone().write_owned().await);
    let snapshot_config = config.clone();
    ctx.spawn_blocking(move || {
        let _guard = LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for file in partitions(&snapshot_config)? {
            queue(&file)
                .clear_stale(state.gate.process(), fencing_generation)
                .map_err(map_pending)?;
        }
        Ok::<_, MemoryError>(())
    })
    .await
    .map_err(|_| unavailable())??;
    tracing::debug!("[memory:tool_writes] erase fence in force; prior pending writes cleared");
    Ok(fence)
}

/// Starts a scoped background drain, retaining config only in memory.
///
/// `root` is the layout root the write was queued under (`CallFacts::reach`).
pub(super) fn schedule(
    ctx: &crate::core::runtime::DispatchCtx,
    config: Arc<Config>,
    root: &Namespace,
) {
    let file = queue_path(&config, root);
    schedule_partition(ctx, config, file);
}

/// Retries all durable partitions belonging to this owner on auth/cron ticks.
pub(super) fn schedule_all(ctx: &crate::core::runtime::DispatchCtx, config: Arc<Config>) {
    if super::engine::switched_off(&config) {
        return;
    }
    let scan_ctx = ctx.clone();
    ctx.spawn(async move {
        let scan_config = config.clone();
        let files = match scan_ctx
            .spawn_blocking(move || partitions(&scan_config))
            .await
        {
            Ok(Ok(files)) => files,
            _ => {
                tracing::debug!(
                    code = "QUEUE_UNAVAILABLE",
                    "[memory:tool_writes] retry discovery deferred"
                );
                return;
            }
        };
        for file in files {
            schedule_partition(&scan_ctx, config.clone(), file);
        }
    });
}

fn schedule_partition(ctx: &crate::core::runtime::DispatchCtx, config: Arc<Config>, file: PathBuf) {
    let owner = owner_state(&config);
    if owner.gate.is_fenced() {
        return;
    }
    let generation = owner.gate.generation();
    let Ok(owner_guard) = owner.barrier.clone().try_read_owned() else {
        return;
    };
    let state = drain_state(&file);
    let Ok(guard) = state.barrier.clone().try_lock_owned() else {
        return;
    };
    ctx.spawn(async move {
        let _guard = guard;
        let _owner_guard = owner_guard;
        let _state = state;
        drain_locked(&config, &file, owner, generation).await;
    });
}

/// Retries oldest first, retaining a failed write and all later writes.
/// Producers hold no queue lock during remote I/O. Also used by auth/cron
/// after restart; the persisted owner binding must still match the active user.
#[cfg(test)]
pub(crate) async fn drain(config: &Config) -> usize {
    let file = queue_path(
        config,
        super::scope::MemoryIdentity::root().resolve(config).root(),
    );
    let owner = owner_state(config);
    let _owner = owner.barrier.read().await;
    let state = drain_state(&file);
    let _draining = state.barrier.lock().await;
    let generation = owner.gate.generation();
    drain_locked(config, &file, owner, generation).await
}

/// Runs one queued write against the bound engine, bounded by [`WRITE_TIMEOUT`].
struct Executor<'a> {
    config: &'a Config,
}

#[async_trait]
impl WriteExecutor for Executor<'_> {
    async fn run(&self, operation: &WriteOperation) -> Result<(), String> {
        let config = self.config;
        let work = async {
            match operation {
                WriteOperation::Learn { item } => {
                    let bound = super::engine::resolve(config).engine()?;
                    let receipt =
                        super::ops::store_on_with(&bound, item.clone(), WriteOptions::accepted())
                            .await?;
                    let category = match item {
                        StoreItem::Learning { kind, .. } => kind.as_str(),
                        _ => "learning",
                    };
                    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::MemoryStored {
                        key: receipt.id.0,
                        category: category.to_string(),
                        namespace: "learnings".to_string(),
                        // A queued write runs off the turn: no chat to show it in.
                        thread_id: None,
                        client_id: None,
                    });
                    Ok(())
                }
                WriteOperation::Forget { ids, reach } => super::ops::forget(
                    config,
                    ForgetParams {
                        ids: ids.clone(),
                        reach: Some(reach.clone()),
                    },
                )
                .await
                .map(|_| ()),
            }
        };
        match tokio::time::timeout(WRITE_TIMEOUT, work).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(error.code().to_string()),
            Err(_) => Err("TIMEOUT".to_string()),
        }
    }
}

async fn drain_locked(config: &Config, file: &Path, state: &OwnerState, generation: u64) -> usize {
    if super::engine::switched_off(config) {
        return 0;
    }
    let active = binding(config);
    queue(file)
        .drain(
            &Executor { config },
            &state.gate,
            generation,
            |entry| entry.binding == active && queue_path(config, &entry.scope_root) == file,
            |_| super::lifecycle::prefetch::invalidate(config),
        )
        .await
}

#[cfg(test)]
#[path = "tool_writes_tests.rs"]
mod tests;
