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
//!   the event, and an offer that gives up later does not run it directly;
//! - an offer that gives up first takes the claim for the direct path and
//!   records it on the host's storage, and every later `Deliver` of the
//!   event skips it (publishing `InboundEventDropped`), across a restart too.
//!
//! | Collection | Document id | Holds |
//! | --- | --- | --- |
//! | `inbound.direct` | `Key::of([source:event_id])` | an event the direct path ran, and when |
//!
//! Claims belong to a workspace: each workspace's are kept apart, in memory
//! and in that workspace's own storage, so no profile reads or writes
//! another's records.
//!
//! Records are kept as long as the module keeps its tombstones (30 days),
//! so a dead-lettered copy redriven later is still recognised. They are
//! rare: only an offer whose every attempt was ambiguous writes one.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, OnceLock, PoisonError};

use serde::{Deserialize, Serialize};
use tinystoragedrivers::{CollectionSpec, ErrorKind, Precondition, Query};

use tinyeventqueues_bus::Turn;

use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::storage::documents::Repo;
use crate::storage::schema_support::key_of;
use crate::storage::DocumentStoreExt;

/// The collection of events the direct path ran.
pub const DIRECT: &str = "inbound.direct";
const DOMAIN: &str = "inbound::claims";

/// How many direct-path events are remembered in memory.
const CAPACITY: usize = 4_096;

/// How long a direct-path event is recorded: the module's tombstone TTL.
const TTL_MS: i64 = 30 * 24 * 60 * 60 * 1_000;

/// Why an event the direct path ran is not delivered.
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
    /// The queues delivered it while the offer waited.
    Delivered,
    /// The direct path runs it.
    Direct,
}

/// An event the direct path ran, as stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    key: String,
    recorded_ms: i64,
}

#[derive(Debug, Default)]
struct Memory {
    claims: HashMap<String, Claim>,
    /// The `Direct` claims, oldest first, for eviction.
    direct: VecDeque<String>,
}

impl Memory {
    fn direct(&mut self, key: &str) {
        if self.claims.insert(key.to_owned(), Claim::Direct) != Some(Claim::Direct) {
            self.direct.push_back(key.to_owned());
        }
        while self.direct.len() > CAPACITY {
            if let Some(oldest) = self.direct.pop_front() {
                self.claims.remove(&oldest);
            }
        }
    }
}

/// The claims of one workspace, over its storage once attached.
#[derive(Default)]
pub struct Claims {
    memory: Mutex<Memory>,
    store: OnceLock<Option<Repo>>,
}

impl std::fmt::Debug for Claims {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Claims")
            .field("attached", &self.store.get().is_some_and(Option::is_some))
            .finish_non_exhaustive()
    }
}

/// Each workspace's claims, by workspace directory.
static WORKSPACES: LazyLock<Mutex<HashMap<PathBuf, Arc<Claims>>>> = LazyLock::new(Mutex::default);

impl Claims {
    /// The claims the offers and deliveries of `config`'s workspace share,
    /// attached to its storage (and loaded, at `now_ms`) on first use.
    #[must_use]
    pub fn of_workspace(config: &Config, now_ms: i64) -> Arc<Self> {
        let claims = Self::for_dir(&config.workspace_dir);
        claims.attach(config, now_ms);
        claims
    }

    fn for_dir(workspace_dir: &Path) -> Arc<Self> {
        Arc::clone(
            WORKSPACES
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(workspace_dir.to_path_buf())
                .or_default(),
        )
    }

    /// Claims over `scoped` storage, loaded at `now_ms` (tests).
    #[cfg(test)]
    pub(crate) fn over(scoped: &crate::storage::Storage, now_ms: i64) -> Self {
        let claims = Self::default();
        claims.attach_repo(Repo::over(scoped, DOMAIN, collection_specs), now_ms);
        claims
    }

    fn memory(&self) -> std::sync::MutexGuard<'_, Memory> {
        self.memory.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Record direct-path events on `config`'s workspace storage from now
    /// on, and remember the ones recorded before (a previous process's).
    /// Only the first call opens the store; a store that cannot be opened
    /// leaves the claims in memory.
    fn attach(&self, config: &Config, now_ms: i64) {
        if self.store.get().is_some() {
            return;
        }
        match Repo::current_for(&config.workspace_dir, DOMAIN, collection_specs) {
            Ok(repo) => self.attach_repo(repo, now_ms),
            Err(error) => {
                tracing::warn!(error = %error, "[inbound::claims] store unavailable; claims kept in memory only");
                let _ = self.store.set(None);
            }
        }
    }

    fn attach_repo(&self, repo: Repo, now_ms: i64) {
        if self.store.set(Some(repo.clone())).is_err() {
            return;
        }
        match load(&repo, now_ms) {
            Ok(keys) => {
                let mut memory = self.memory();
                for key in keys {
                    memory.direct(&key);
                }
            }
            Err(error) => {
                tracing::warn!(error = %error, "[inbound::claims] could not read earlier direct-path events");
            }
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
        if matches!(memory.claims.get(key), Some(Claim::Open | Claim::Delivered)) {
            memory.claims.remove(key);
        }
    }

    /// Every attempt to offer `key` was ambiguous: take it for the direct
    /// path at `now_ms`. `false` when the queues delivered it meanwhile, so
    /// the caller must not run it.
    pub fn take_direct(&self, key: &str, now_ms: i64) -> bool {
        {
            let mut memory = self.memory();
            if memory.claims.get(key) == Some(&Claim::Delivered) {
                memory.claims.remove(key);
                return false;
            }
            memory.direct(key);
        }
        if let Some(Some(repo)) = self.store.get() {
            if let Err(error) = record(repo, key, now_ms) {
                // Still skipped in this process; only a restart before the
                // queued copy is delivered could run it a second time.
                tracing::warn!(key = %key, error = %error, "[inbound::claims] could not record a direct-path event");
            }
        }
        true
    }

    /// The queues deliver `key`: `true` when the direct path ran it, so the
    /// delivery skips it.
    pub fn delivering(&self, key: &str) -> bool {
        let mut memory = self.memory();
        match memory.claims.get(key) {
            Some(Claim::Direct) => true,
            Some(Claim::Open) => {
                memory.claims.insert(key.to_owned(), Claim::Delivered);
                false
            }
            Some(Claim::Delivered) | None => false,
        }
    }
}

impl Claims {
    /// Leave out of `turn`, which the queues deliver, the events the direct
    /// path ran, each dropped as `handled_directly`.
    pub fn leave_out_direct(&self, turn: &mut Turn) {
        let turn_id = turn.turn_id.clone();
        turn.events.retain(|event| {
            if !self.delivering(&event.dedupe_key()) {
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
