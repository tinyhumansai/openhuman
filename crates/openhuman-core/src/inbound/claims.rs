//! Who runs an event whose hand-off to the queues was ambiguous: the
//! caller's direct path or the queues' `Deliver`.
//!
//! [`super::route::offer_or_direct`] falls back to its caller's direct path
//! when every `Ingest` attempt was ambiguous (a timeout, a transport fault),
//! yet the module may have persisted the event all the same and deliver it
//! later. Without a record that event is triaged twice. The claims decide
//! which side runs it, by the event's dedupe key (`source:event_id`):
//!
//! - the offer opens a claim before its first attempt and closes it when the
//!   queues answer;
//! - a `Deliver` of an event whose claim is open takes it: the queues run
//!   the event, and no ambiguous offer of it runs it directly any more;
//! - an offer that gives up first takes the event for the direct path
//!   ([`DirectClaim`]), once: a second ambiguous offer of the same event is
//!   not given it. While the direct path runs, a `Deliver` of the event is
//!   asked to come back later; when it handled the event the claim is
//!   recorded on the host's storage, and every later `Deliver` skips the
//!   event (publishing `InboundEventDropped`), across a restart too. When it
//!   did not (a failed or deferred triage, a dropped task), the claim is
//!   released and the queues' copy runs, so nothing is lost.
//!
//! | Collection | Document id | Holds |
//! | --- | --- | --- |
//! | `inbound.direct` | `Key::of([source:event_id])` | an event the direct path handled, and when |
//!
//! Claims belong to a workspace and storage scope: each one's are kept
//! apart, in memory and in that scope's own storage, so no profile reads or
//! writes another's records. The records are read before the claims answer
//! anything; a store that cannot be read is tried again on the next use,
//! the claims answering from memory meanwhile, so a broken store never stops
//! delivery.
//!
//! Records are kept as long as the module keeps its tombstones (30 days),
//! so a dead-lettered copy redriven later is still recognised. They are
//! rare: only a direct path run after an ambiguous offer writes one.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use tinystoragedrivers::{CollectionSpec, ErrorKind, Precondition, Query};

use tinyeventqueues_bus::Turn;

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::core::runtime::DispatchCtx;
use crate::storage::documents::Repo;
use crate::storage::schema_support::key_of;
use crate::storage::DocumentStoreExt;

/// The collection of events the direct path handled.
pub const DIRECT: &str = "inbound.direct";
const DOMAIN: &str = "inbound::claims";

/// How many settled claims (delivered, or handled directly) are remembered
/// in memory.
const CAPACITY: usize = 4_096;

/// How long a direct-path event is recorded: the module's tombstone TTL.
const TTL_MS: i64 = 30 * 24 * 60 * 60 * 1_000;

/// Why an event the direct path handled is not delivered.
pub const HANDLED_DIRECTLY: &str = "handled_directly";

/// What the inbound host keeps for its claims.
pub fn collection_specs() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new(DIRECT)]
}

/// Where an event's claim stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Claim {
    /// The offer is still waiting for the queues' answer.
    Open,
    /// The queues delivered it: they own it.
    Delivered,
    /// The direct path is running it.
    Direct,
    /// The direct path handled it.
    Handled,
}

/// What a `Deliver` does with one event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Deliver it.
    Run,
    /// The direct path is running it: deliver it later.
    Wait,
    /// The direct path handled it: drop it.
    Skip,
}

/// An event the direct path handled, as stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    key: String,
    recorded_ms: i64,
}

#[derive(Debug, Default)]
struct Memory {
    claims: HashMap<String, Claim>,
    /// The settled claims, oldest first, for eviction.
    settled: VecDeque<String>,
    /// Whether a handled claim was evicted, so a miss may be in the store.
    evicted: bool,
}

impl Memory {
    fn settle(&mut self, key: &str, claim: Claim) {
        if !matches!(
            self.claims.insert(key.to_owned(), claim),
            Some(Claim::Delivered | Claim::Handled)
        ) {
            self.settled.push_back(key.to_owned());
        }
        while self.settled.len() > CAPACITY {
            if let Some(oldest) = self.settled.pop_front() {
                if self.claims.remove(&oldest) == Some(Claim::Handled) {
                    self.evicted = true;
                }
            }
        }
    }
}

/// Where the claims' records live.
#[derive(Clone)]
enum Store {
    /// Nowhere: the claims are memory only.
    None,
    /// In this workspace's storage, not read yet.
    Unread(PathBuf),
    /// In this repository, read into memory.
    Read(Repo),
}

/// The claims of one workspace and storage scope.
pub struct Claims {
    memory: Mutex<Memory>,
    store: Mutex<Store>,
}

impl Default for Claims {
    /// Claims in memory only.
    fn default() -> Self {
        Self::with_store(Store::None)
    }
}

impl std::fmt::Debug for Claims {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let read = matches!(*self.store(), Store::Read(_));
        f.debug_struct("Claims")
            .field("read", &read)
            .finish_non_exhaustive()
    }
}

/// Claims by workspace directory and storage scope.
type ByWorkspace = HashMap<(PathBuf, String), Arc<Claims>>;

/// Each workspace and storage scope's claims.
static WORKSPACES: LazyLock<Mutex<ByWorkspace>> = LazyLock::new(Mutex::default);

impl Claims {
    fn with_store(store: Store) -> Self {
        Self {
            memory: Mutex::new(Memory::default()),
            store: Mutex::new(store),
        }
    }

    /// The claims the offers and deliveries of `workspace_dir` share under
    /// the acting storage scope, their records read (at `now_ms`) on first
    /// use and again after a failed read.
    #[must_use]
    pub fn of_workspace(workspace_dir: &Path, now_ms: i64) -> Arc<Self> {
        let scope = crate::storage::current_scope()
            .map(|scope| scope.as_str().to_owned())
            .unwrap_or_default();
        let claims = Arc::clone(
            WORKSPACES
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry((workspace_dir.to_path_buf(), scope))
                .or_insert_with(|| {
                    Arc::new(Self::with_store(Store::Unread(workspace_dir.to_path_buf())))
                }),
        );
        claims.read(now_ms);
        claims
    }

    /// Claims over `scoped` storage, read at `now_ms` (tests).
    #[cfg(test)]
    pub(crate) fn over(scoped: &crate::storage::Storage, now_ms: i64) -> Self {
        let repo = Repo::over(scoped, DOMAIN, collection_specs);
        let claims = Self::with_store(Store::None);
        let keys = load(&repo, now_ms).expect("read the claims");
        claims.remember(keys);
        *claims.store() = Store::Read(repo);
        claims
    }

    fn memory(&self) -> std::sync::MutexGuard<'_, Memory> {
        self.memory.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Read the records into memory unless they were. The store stays locked
    /// until they are, so no caller answers from a half-read memory.
    fn read(&self, now_ms: i64) {
        let mut store = self.store();
        let Store::Unread(workspace_dir) = &*store else {
            return;
        };
        let read = Repo::current_for(workspace_dir, DOMAIN, collection_specs)
            .and_then(|repo| load(&repo, now_ms).map(|keys| (repo, keys)));
        match read {
            Ok((repo, keys)) => {
                self.remember(keys);
                *store = Store::Read(repo);
            }
            Err(error) => {
                tracing::warn!(error = %error, "[inbound::claims] records unreadable; answering from memory and reading again next time");
            }
        }
    }

    fn remember(&self, keys: Vec<String>) {
        let mut memory = self.memory();
        for key in keys {
            memory.settle(&key, Claim::Handled);
        }
    }

    fn repo(&self) -> Option<Repo> {
        match &*self.store() {
            Store::Read(repo) => Some(repo.clone()),
            Store::None | Store::Unread(_) => None,
        }
    }

    /// An offer of `key` is about to be sent.
    pub fn open(&self, key: &str) {
        self.memory()
            .claims
            .entry(key.to_owned())
            .or_insert(Claim::Open);
    }

    /// The queues answered the offer of `key`: they own the event, or
    /// proved they never saw it.
    pub fn close(&self, key: &str) {
        let mut memory = self.memory();
        if memory.claims.get(key) == Some(&Claim::Open) {
            memory.claims.remove(key);
        }
    }

    /// Every attempt to offer `key` was ambiguous: take it for the direct
    /// path. `None` when the queues delivered it, or another offer has the
    /// direct path running or done, so the caller must not run it.
    pub fn take_direct(self: &Arc<Self>, key: &str) -> Option<DirectClaim> {
        let mut memory = self.memory();
        match memory.claims.get(key) {
            Some(Claim::Delivered | Claim::Direct | Claim::Handled) => None,
            Some(Claim::Open) | None => {
                memory.claims.insert(key.to_owned(), Claim::Direct);
                Some(DirectClaim {
                    claims: Arc::clone(self),
                    key: key.to_owned(),
                    finished: false,
                })
            }
        }
    }

    /// The queues deliver `key`.
    pub fn delivering(&self, key: &str) -> Delivery {
        let mut memory = self.memory();
        match memory.claims.get(key) {
            Some(Claim::Handled) => Delivery::Skip,
            Some(Claim::Direct) => Delivery::Wait,
            Some(Claim::Open) => {
                memory.settle(key, Claim::Delivered);
                Delivery::Run
            }
            Some(Claim::Delivered) => Delivery::Run,
            None if memory.evicted => {
                drop(memory);
                if self.recorded(key) {
                    self.memory().settle(key, Claim::Handled);
                    Delivery::Skip
                } else {
                    Delivery::Run
                }
            }
            None => Delivery::Run,
        }
    }

    /// Whether the store holds an unexpired record of `key` (a claim that
    /// left memory).
    fn recorded(&self, key: &str) -> bool {
        let Some(repo) = self.repo() else {
            return false;
        };
        let Ok(id) = key_of(key) else {
            return false;
        };
        let now_ms = chrono::Utc::now().timestamp_millis();
        let found = repo.run(|docs| async move { docs.get(DIRECT, &id).await });
        match found {
            Ok(Some(found)) => serde_json::from_value::<Record>(found.doc)
                .is_ok_and(|record| now_ms - record.recorded_ms <= TTL_MS),
            Ok(None) => false,
            Err(error) => {
                tracing::warn!(key = %key, error = %error, "[inbound::claims] could not read a direct-path record; delivering");
                false
            }
        }
    }

    /// Settle `key`'s direct-path claim: recorded at `now_ms` when the
    /// direct path handled the event, released otherwise.
    fn finish(&self, key: &str, handled: bool, now_ms: i64) {
        {
            let mut memory = self.memory();
            if memory.claims.get(key) != Some(&Claim::Direct) {
                return;
            }
            if handled {
                memory.settle(key, Claim::Handled);
            } else {
                memory.claims.remove(key);
            }
        }
        if !handled {
            return;
        }
        if let Some(repo) = self.repo() {
            if let Err(error) = record(&repo, key, now_ms) {
                // Still skipped in this process; only a restart before the
                // queued copy is delivered could run it a second time.
                let detail =
                    format!("[inbound::claims] could not record a direct-path event: {error:#}");
                crate::core::observability::report_error_or_expected(
                    detail.as_str(),
                    "inbound",
                    "record_direct",
                    &[],
                );
            }
        }
    }

    /// The claims of the workspace `ctx` is bound to now (the active
    /// workspace can change without the core being rebuilt), read at
    /// `now_ms` on first use.
    pub async fn of_ctx(ctx: &DispatchCtx, now_ms: i64) -> Arc<Self> {
        let bound = ctx.core().and_then(|core| core.workspace_dir().ok());
        let workspace_dir = match bound {
            Some(dir) => dir,
            None => match ctx.load_config().await {
                Ok(config) => config.workspace_dir,
                Err(error) => {
                    tracing::debug!(error = %error, "[inbound::claims] config unavailable; the default workspace's claims");
                    crate::config::Config::default().workspace_dir
                }
            },
        };
        Self::of_workspace(&workspace_dir, now_ms)
    }

    /// What becomes of `turn`, which the queues deliver: delivered later
    /// while a direct path is still running one of its events; otherwise
    /// without the events the direct paths handled, each dropped as
    /// `handled_directly`. A turn that came with no events is delivered as
    /// it is (and rejected there).
    pub fn sort(&self, mut turn: Turn) -> Sorted {
        if turn.events.is_empty() {
            return Sorted::Deliver(turn);
        }
        let deliveries: Vec<Delivery> = turn
            .events
            .iter()
            .map(|event| self.delivering(&event.dedupe_key()))
            .collect();
        if deliveries.contains(&Delivery::Wait) {
            tracing::info!(turn = %turn.turn_id, "[inbound] the direct path is still running an event of this turn; delivering it later");
            return Sorted::Wait(turn);
        }
        let turn_id = turn.turn_id.clone();
        let mut deliveries = deliveries.into_iter();
        turn.events.retain(|event| {
            if deliveries.next() != Some(Delivery::Skip) {
                return true;
            }
            tracing::info!(
                turn = %turn_id,
                source = %event.source,
                event_id = %event.event_id,
                "[inbound] event already handled on the direct path; not delivering it"
            );
            BUS.publish(DomainEvent::InboundEventDropped {
                source: event.source.clone(),
                event_id: event.event_id.clone(),
                reason: HANDLED_DIRECTLY.to_owned(),
            });
            false
        });
        if turn.events.is_empty() {
            Sorted::Handled
        } else {
            Sorted::Deliver(turn)
        }
    }
}

/// What becomes of a turn given the direct paths' claims on its events.
#[derive(Debug)]
pub enum Sorted {
    /// Deliver what is left of it.
    Deliver(Turn),
    /// A direct path is running one of its events: deliver it later.
    Wait(Turn),
    /// The direct paths handled every event.
    Handled,
}

/// An event taken for the direct path. Its caller reports whether the
/// direct path handled it ([`Self::finish`]); dropped unreported, it is
/// released, so the queues' copy runs.
#[derive(Debug)]
pub struct DirectClaim {
    claims: Arc<Claims>,
    key: String,
    finished: bool,
}

impl DirectClaim {
    /// The direct path ended: `handled` when it decided the event (applied
    /// or terminal), so the queues' copy is dropped; otherwise it is
    /// released and the queues' copy runs.
    pub fn finish(mut self, handled: bool) {
        self.finished = true;
        self.claims
            .finish(&self.key, handled, chrono::Utc::now().timestamp_millis());
    }
}

impl Drop for DirectClaim {
    fn drop(&mut self) {
        if !self.finished {
            self.claims.finish(&self.key, false, 0);
        }
    }
}

/// The keys recorded within the TTL at `now_ms`; older records are deleted.
fn load(repo: &Repo, now_ms: i64) -> anyhow::Result<Vec<String>> {
    repo.run(|docs| async move {
        let mut keys = Vec::new();
        for found in docs.query_all(DIRECT, &Query::all()).await? {
            let Ok(record) = serde_json::from_value::<Record>(found.doc.clone()) else {
                continue;
            };
            if now_ms - record.recorded_ms > TTL_MS {
                match docs.delete(DIRECT, &found.id, found.unchanged()).await {
                    Ok(_) => {}
                    Err(error) if error.kind() == ErrorKind::Conflict => {}
                    Err(error) => return Err(error),
                }
            } else {
                keys.push(record.key);
            }
        }
        Ok(keys)
    })
}

fn record(repo: &Repo, key: &str, now_ms: i64) -> anyhow::Result<()> {
    let id = key_of(key)?;
    let doc = serde_json::to_value(Record {
        key: key.to_owned(),
        recorded_ms: now_ms,
    })?;
    repo.run(|docs| async move {
        docs.put(DIRECT, &id, doc, Precondition::None).await?;
        Ok(())
    })
}

#[cfg(test)]
#[path = "claims_tests.rs"]
mod tests;
