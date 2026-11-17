//! Durable, bounded FIFO for the agent tool's optional memory writes.
//!
//! The queue itself (bounds, coalescing, fencing, ordered drain) is
//! `tinymemory_tools::pending`; this module is the host adapter. It keeps the
//! per-owner partitioning of the files under `<workspace>/memory/`, the
//! validation and scrubbing of a write before it is queued, the
//! `ListStore` over the memory state store (one `tsd.kv` value per
//! partition, named like the old `<workspace>/memory/<name>.json`), the executor that runs a write against
//! the bound engine under a timeout, and the erasure barrier that waits for a
//! write already in flight. Only scrubbed items and trusted scope are
//! persisted; credentials are resolved at drain time. Enqueue never waits on
//! the remote engine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tinymemory_api::{Namespace, Reach, StoreItem, WriteOptions};
use tinymemory_tools::confine::Policy;
use tinymemory_tools::list_store::{ListStore, StoreError};
use tinymemory_tools::pending::{
    Ack, Binding, Entry, Epoch, FenceToken, Limits, PendingError, PendingWrites, WriteExecutor,
    WriteGate, WriteOperation,
};
use tinymemory_tools::ToolScope;
use tinymemory_tools::WriteSink;

use super::types::ForgetParams;
use crate::config::Config;
use tinymemory_api::{Error, Result};

const MAX_PENDING: usize = 256;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const WRITE_TIMEOUT: Duration = Duration::from_secs(120);
const QUEUED: &str = "queued; not yet saved to memory";
static LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Default)]
struct DrainState {
    barrier: Arc<tokio::sync::Mutex<()>>,
}

/// One owner's erasure state: the module's gate (generation, fences) and the
/// barrier an erasure takes exclusively to wait for the write in flight.
/// Kept alive by active mutations and fences; the registry only holds a weak
/// reference so owners with no active work can be released.
struct OwnerState {
    gate: WriteGate,
    barrier: Arc<tokio::sync::RwLock<()>>,
}

type Owners = HashMap<PathBuf, Weak<OwnerState>>;
static OWNERS: LazyLock<Mutex<Owners>> = LazyLock::new(|| Mutex::new(HashMap::new()));

type DrainLocks = HashMap<PathBuf, std::sync::Weak<DrainState>>;
static DRAINS: LazyLock<Mutex<DrainLocks>> = LazyLock::new(|| Mutex::new(HashMap::new()));

type Queue = PendingWrites<FileStore>;
static QUEUES: LazyLock<Mutex<HashMap<PathBuf, Weak<Queue>>>> =
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
        config.memory.layout == crate::config::MemoryLayoutMode::V3,
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
        config.memory.layout == crate::config::MemoryLayoutMode::V3,
        root,
    ))
    .expect("partition fields serialize");
    config.workspace_dir.join("memory").join(format!(
        "{}-{:x}.json",
        owner_prefix(config),
        Sha256::digest(partition)
    ))
}

fn unavailable() -> Error {
    Error::Unavailable("memory write queue unavailable; durable queueing was not confirmed".into())
}

/// The host policy and queue adapter used by TinyMemory's single tool.
pub(super) struct HostWriteSink {
    config: Arc<Config>,
}

impl HostWriteSink {
    pub(super) fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

impl WriteSink for HostWriteSink {
    fn preflight(&self, _scope: &ToolScope) -> Result<()> {
        if super::engine::switched_off(&self.config) {
            return Err(Error::Off("memory is disabled".into()));
        }
        super::ops::saas_confinement(&self.config, Policy::Clamp)?;
        Ok(())
    }

    fn enqueue(&self, operation: WriteOperation, scope: &ToolScope) -> Result<Ack> {
        enqueue_operation(&self.config, operation, scope)
    }
}

fn enqueue_operation(config: &Config, operation: WriteOperation, scope: &ToolScope) -> Result<Ack> {
    if super::engine::switched_off(config) {
        return Err(Error::Off("memory is disabled".into()));
    }
    let confinement = super::ops::saas_confinement(config, Policy::Clamp)?;
    let mut reach = scope
        .reach
        .clone()
        .unwrap_or_else(|| Reach::subtree(scope.place.clone()));
    if let Some(confinement) = &confinement {
        reach = confinement.reach(Some(reach))?;
    }
    let scope_root = reach.at.clone();
    let operation = match operation {
        WriteOperation::Learn { mut item } => {
            if let Some(confinement) = &confinement {
                confinement.item(&mut item)?;
            }
            if reach.inherit || !Reach::subtree(reach.at.clone()).admits(&item.meta().namespace) {
                return Err(Error::InvalidRequest(
                    "learning namespace is outside the trusted reach".into(),
                ));
            }
            WriteOperation::Learn {
                item: tinymemory_integrations::safety::scrub_item_with(
                    item,
                    crate::memory::host_policy::host_policy(),
                )
                .value,
            }
        }
        WriteOperation::Forget { ids, .. } => WriteOperation::Forget { ids, reach },
    };
    let file = queue_path(config, &scope_root);
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !file.exists() && partitions(config)?.len() >= MAX_PENDING {
        return Err(Error::Unavailable(
            "memory write queue full; nothing was queued".into(),
        ));
    }
    let owner = owner_state(config);
    queue(&file)
        .enqueue(operation, scope_root, binding(config), &owner.gate)
        .map_err(map_pending)
}

/// A [`ListStore`] over one queue partition (one storage write per save).
struct FileStore {
    file: PathBuf,
}

impl ListStore<Entry> for FileStore {
    fn load(&self) -> std::result::Result<Vec<Entry>, StoreError> {
        read_file(&self.file).map_err(|_| StoreError("queue file unreadable".into()))
    }

    fn save(&self, entries: &[Entry]) -> std::result::Result<(), StoreError> {
        write_file(&self.file, entries).map_err(|_| StoreError("queue file unwritable".into()))
    }
}

/// The state-store coordinates of a queue partition: its workspace and name
/// (the stem of its legacy `<workspace>/memory/<name>.json` path).
fn coordinates(file: &Path) -> std::io::Result<(&Path, &str)> {
    let workspace = file
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| std::io::Error::other("the queue partition has no workspace"))?;
    let name = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| std::io::Error::other("the queue partition has no name"))?;
    Ok((workspace, name))
}

fn read_file(path: &Path) -> std::io::Result<Vec<Entry>> {
    let (workspace, name) = coordinates(path)?;
    let stored: Option<Vec<Stored>> =
        super::state_store::load_strict(workspace, name).map_err(std::io::Error::other)?;
    let stored = stored.unwrap_or_default();
    Ok(stored.into_iter().map(Entry::from).collect())
}

fn write_file(file: &Path, entries: &[Entry]) -> std::io::Result<()> {
    let (workspace, name) = coordinates(file)?;
    if serde_json::to_vec(entries)
        .map_err(std::io::Error::other)?
        .len()
        > MAX_BYTES
    {
        return Err(std::io::Error::other("queue too large"));
    }
    super::state_store::save(workspace, name, &entries).map_err(std::io::Error::other)
}

fn queue(file: &Path) -> Arc<Queue> {
    let mut queues = QUEUES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    queues.retain(|_, queue| queue.strong_count() > 0);
    if let Some(queue) = queues.get(file).and_then(Weak::upgrade) {
        return queue;
    }
    let queue = Arc::new(PendingWrites::new(
        FileStore {
            file: file.to_path_buf(),
        },
        Limits {
            max_entries: MAX_PENDING,
            max_bytes: MAX_BYTES,
        },
    ));
    queues.insert(file.to_path_buf(), Arc::downgrade(&queue));
    queue
}

fn map_pending(error: PendingError) -> Error {
    Error::Unavailable(error.to_string())
}

fn read_at(path: &Path) -> Result<Vec<Entry>> {
    queue(path).entries().map_err(map_pending)
}

#[cfg(test)]
fn write_at(file: &Path, entries: &[Entry]) -> Result<()> {
    FileStore {
        file: file.to_path_buf(),
    }
    .save(entries)
    .map_err(|_| unavailable())
}

#[cfg(test)]
fn read(workspace: &Path) -> Result<Vec<Entry>> {
    read_at(&path(workspace))
}
#[cfg(test)]
fn write(workspace: &Path, entries: &[Entry]) -> Result<()> {
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

fn owner_state(config: &Config) -> Arc<OwnerState> {
    let key = config
        .workspace_dir
        .join("memory")
        .join(owner_prefix(config));
    let mut owners = OWNERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    owners.retain(|_, owner| owner.strong_count() > 0);
    if let Some(owner) = owners.get(&key).and_then(Weak::upgrade) {
        return owner;
    }
    let owner = Arc::new(OwnerState {
        // A new gate starts at generation zero, so it needs a distinct epoch
        // identity to make entries from a released gate stale after eviction.
        gate: WriteGate::new(uuid::Uuid::new_v4().to_string()),
        barrier: Arc::default(),
    });
    owners.insert(key, Arc::downgrade(&owner));
    owner
}

fn partitions(config: &Config) -> Result<Vec<PathBuf>> {
    let prefix = format!("{}-", owner_prefix(config));
    let names =
        super::state_store::names(&config.workspace_dir, &prefix).map_err(|_| unavailable())?;
    if names.len() > MAX_PENDING {
        return Err(unavailable());
    }
    Ok(names
        .into_iter()
        .map(|name| super::state_store::legacy_path(&config.workspace_dir, &name))
        .collect())
}

/// Owner epoch captured when automatic turn work is admitted, before it can
/// wait in the prefetch queue. Erasure retires older work and waits for active
/// mutations while subsequent work waits behind the exclusive barrier.
pub(crate) struct AutomaticMutation {
    state: Arc<OwnerState>,
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
    let generation = state.gate.generation();
    AutomaticMutation { state, generation }
}

/// Holds the owner's mutation barrier through explicit engine erasure.
/// Subsequent queued writes resume only after the outer erasure finishes.
pub(crate) struct WriteFence {
    guard: Option<tokio::sync::OwnedRwLockWriteGuard<()>>,
    token: Option<OwnedFenceToken>,
    state: Arc<OwnerState>,
    config: Arc<Config>,
    ctx: crate::core::runtime::DispatchCtx,
}

/// An owned fence token. The token borrows its gate, so keep the owner alive
/// and drop the token before releasing that owner.
struct OwnedFenceToken {
    _state: Arc<OwnerState>,
    token: Option<FenceToken<'static>>,
}

impl OwnedFenceToken {
    fn new(state: Arc<OwnerState>) -> Self {
        let token = state.gate.begin_fence();
        // SAFETY: `state` owns the borrowed gate and remains alive in this
        // wrapper. `Drop` takes the token before Rust drops the state field.
        let token = unsafe { std::mem::transmute::<FenceToken<'_>, FenceToken<'static>>(token) };
        Self {
            _state: state,
            token: Some(token),
        }
    }

    fn generation(&self) -> u64 {
        self.token.as_ref().expect("live fence token").generation()
    }
}

impl Drop for OwnedFenceToken {
    fn drop(&mut self) {
        drop(self.token.take());
    }
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
) -> Result<WriteFence> {
    let state = owner_state(config);
    let token = OwnedFenceToken::new(state.clone());
    let fencing_generation = token.generation();
    let mut fence = WriteFence {
        guard: None,
        token: Some(token),
        state: state.clone(),
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
        Ok::<_, Error>(())
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
        drain_locked(&config, &file, &owner, generation).await;
    });
}

/// Retries oldest first, retaining a failed write and all later writes.
/// Producers hold no queue lock during remote I/O. Also used by auth/cron
/// after restart; the persisted owner binding must still match the active user.
#[cfg(test)]
pub(crate) async fn drain(config: &Config) -> usize {
    let file = queue_path(
        config,
        tinymemory_tools::layout::identity::Identity::root()
            .resolve(&crate::memory::scope::identity_pins(config))
            .root(),
    );
    let owner = owner_state(config);
    let _owner = owner.barrier.read().await;
    let state = drain_state(&file);
    let _draining = state.barrier.lock().await;
    let generation = owner.gate.generation();
    drain_locked(config, &file, &owner, generation).await
}

/// Runs one queued write against the bound engine, bounded by [`WRITE_TIMEOUT`].
struct Executor<'a> {
    config: &'a Config,
}

#[async_trait]
impl WriteExecutor for Executor<'_> {
    async fn run(&self, operation: &WriteOperation) -> std::result::Result<(), String> {
        let config = self.config;
        let work = async {
            match operation {
                WriteOperation::Learn { item } => {
                    let bound = super::engine::resolve(config).await.engine()?;
                    let receipt = bound
                        .engine
                        .store_with(item.clone(), WriteOptions::accepted())
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
pub(crate) mod tests;
