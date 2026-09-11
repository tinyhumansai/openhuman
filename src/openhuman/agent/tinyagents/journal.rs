//! Durable event journals + status stores for tinyagents turns (issue #4249,
//! Workstream 05-events, 05.1).
//!
//! The live [`crate::openhuman::agent::tinyagents::observability::OpenhumanEventBridge`]
//! mirrors the harness [`EventSink`] onto openhuman's in-process `AgentProgress`
//! stream — transient state that is lost the moment the UI detaches. This module
//! makes that history **durable**: it attaches, *in addition to* the untouched
//! bridge, a crate [`StoreEventJournal`] (over the same 04-sessions
//! [`JsonlAppendStore`] under `{workspace}/tinyagents_store/journal`) plus a
//! [`HarnessStatusStore`] writer, so a run can be reconstructed after the fact —
//! even for an unobserved (`on_progress = None`) turn.
//!
//! Everything here is **best-effort and non-fatal**: opening the store,
//! subscribing the sink, and every status/journal write swallow errors behind a
//! grep-friendly `[journal]` log line and never fail or alter a chat turn. The
//! existing bridge/global-bus path is left fully intact — this is a pure
//! observer add-on.
//!
//! ## Composition
//!
//! The crate [`EventSink`] is itself the fan-out point: the (already-subscribed)
//! `OpenhumanEventBridge` and this journal sink are independent subscribers, so
//! **both** receive every event. The journal side is wrapped in a
//! [`FanOutSink`] as the durable-observer composition seam (05.2 will add graph
//! sinks here) and its records pass through a [`RedactingSink`] so process
//! credentials are masked before anything is persisted.
//!
//! ## Stable event ids (05.1)
//!
//! The run [`EventSink`] is seeded by the caller with
//! [`EventSink::with_stream_id`]`(run_id)` (see [`mint_run_id`]), so every
//! persisted observation carries a restart-stable `event_id` of the form
//! `{run_id}-evt-{offset}`. That is the id a late-attaching replay reader
//! reconstructs the timeline from — the same `(stream_id, offset)` always mints
//! the same id, and two runs never collide even if both restart their offset
//! counter at zero.
//!
//! ## Follow-ups (not in this slice)
//!
//! - A replay RPC (`agent.run_events`?) that surfaces [`read_run_events`] /
//!   [`read_run_status`] to the desktop for mid-run reconnect (05.x).
//! - Full sub-agent / graph run lineage (`parent_run_id` / `root_run_id`
//!   threading) — wired in 05.2/05.3. This slice threads `thread_id` (from the
//!   sub-agent task scope) so [`FileStatusStore::list_by_thread`] answers.
//!
//! [`EventSink::with_stream_id`]: tinyagents_harness::events::EventSink::with_stream_id

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use once_cell::sync::Lazy;

use tinyagents_harness::error::Result as TaResult;
use tinyagents_harness::events::{EventSink, HarnessRunStatus};
use tinyagents_harness::ids::{ComponentId, HarnessPhase, RunId, ThreadId};
use tinyagents_harness::observability::{
    AgentObservation, FanOutSink, HarnessEventJournal, HarnessStatusStore, JournalSink,
    RedactingSink, StoreEventJournal,
};
use tinyagents_harness::store::{FileStore, Store};

use crate::openhuman::agent::session_import::ops::open_session_stores;

/// KV namespace the durable per-run [`HarnessRunStatus`] snapshots live under
/// (`{workspace}/tinyagents_store/kv/run_status/<run_id>.json`). Slash-free so
/// it round-trips the crate [`FileStore`] name sanitizer.
const STATUS_NS: &str = "run_status";

/// Best-effort live request → durable tinyagents journal stream map. The web
/// progress bridge uses this at turn end to shadow-project spans from the
/// journal and compare them against the live `SpanCollector` path during C4 S3.
static REQUEST_JOURNAL_RUNS: Lazy<Mutex<HashMap<String, String>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Mints a fresh, slash-free, process-unique run id (`run.<32-hex>`), used three
/// ways for one turn: the [`EventSink::with_stream_id`] prefix (so persisted
/// `event_id`s are the restart-stable `{run_id}-evt-{offset}`), the journal
/// stream key, and the status-store key. The `simple()` uuid form (no hyphens)
/// keeps the id inside the crate store's allowed-character set.
///
/// The caller mints this *before* creating the run [`EventSink`] so the same id
/// seeds the sink stream prefix and the durable journal/status — see
/// [`attach_turn_journal`].
///
/// [`EventSink::with_stream_id`]: tinyagents_harness::events::EventSink::with_stream_id
pub(crate) fn mint_run_id() -> RunId {
    RunId::new(format!("run.{}", uuid::Uuid::new_v4().simple()))
}

pub(crate) fn register_request_journal_run(request_id: &str, run_id: &str) {
    match REQUEST_JOURNAL_RUNS.lock() {
        Ok(mut runs) => {
            runs.insert(request_id.to_string(), run_id.to_string());
            log::debug!(
                "[journal] registered request journal request_id={} run_id={}",
                request_id,
                run_id
            );
        }
        Err(err) => {
            log::debug!(
                "[journal] request journal registry poisoned request_id={} err={err}",
                request_id
            );
        }
    }
}

pub(crate) fn take_request_journal_run(request_id: &str) -> Option<String> {
    REQUEST_JOURNAL_RUNS
        .lock()
        .ok()
        .and_then(|mut runs| runs.remove(request_id))
}

/// Resolve the internal workspace directory (`{workspace}`) whose
/// `tinyagents_store/` subtree holds the journal + kv stores. Async because the
/// config load is async; errors are surfaced so callers can log-and-skip.
async fn resolve_workspace() -> anyhow::Result<PathBuf> {
    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| anyhow::anyhow!("[journal] load config for workspace: {e}"))?;
    Ok(config.workspace_dir)
}

/// The OpenHuman journal redaction policy (issue #4249, 05.1).
///
/// The crate [`RedactingSink`] masks configured secret substrings anywhere they
/// appear in a serialized event before the observation is persisted. This policy
/// seeds it with the process's credential material — the values of environment
/// variables whose name looks like a secret (contains `KEY` / `TOKEN` / `SECRET`
/// / `PASSWORD` / `PASSWD` / `CREDENTIAL` / `BEARER`) — so an API key or bearer
/// token echoed into model text, a tool argument fragment, or an error string is
/// never written to the durable journal in the clear. Only reasonably long
/// values (>= 8 chars) are masked, so unrelated short config values are left
/// alone. Minimal but real: it does not persist raw secrets, and structural
/// prompt/PII stripping is a follow-up if the event vocabulary grows to carry
/// full prompts.
fn openhuman_redaction_secrets() -> Vec<String> {
    const MARKERS: [&str; 7] = [
        "KEY",
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "BEARER",
    ];
    let mut secrets = Vec::new();
    for (name, value) in std::env::vars() {
        let upper = name.to_ascii_uppercase();
        if MARKERS.iter().any(|m| upper.contains(m)) && value.trim().len() >= 8 {
            secrets.push(value);
        }
    }
    log::debug!(
        "[journal] redaction policy seeded with {} secret value(s)",
        secrets.len()
    );
    secrets
}

/// A durable [`HarnessStatusStore`] backed by a crate [`FileStore`] KV
/// namespace.
///
/// The crate ships only an in-memory [`HarnessStatusStore`]
/// (`InMemoryStatusStore`), which does not survive a process restart — useless
/// for the "reattach after the UI died and see what is still running" use case
/// this slice targets. This small `Store`-backed impl overwrites one
/// `run_status/<run_id>.json` file per run (compact snapshot: ids, phase,
/// counters, timestamps, error — never prompts or payloads) and answers the
/// lineage/liveness queries by enumerating the namespace via
/// [`Store::list`]. `list_by_root` is what lets a supervisor find "every active
/// descendant of this root run".
pub(crate) struct FileStatusStore {
    kv: FileStore,
}

impl FileStatusStore {
    /// Wrap the workspace kv store as a durable status store.
    pub(crate) fn new(kv: FileStore) -> Self {
        Self { kv }
    }

    /// Enumerate every persisted status snapshot (best-effort per-record decode:
    /// a corrupt/legacy record is skipped, never fatal).
    async fn all(&self) -> TaResult<Vec<HarnessRunStatus>> {
        let keys = self.kv.list(STATUS_NS).await?;
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(value) = self.kv.get(STATUS_NS, &key).await? {
                match serde_json::from_value::<HarnessRunStatus>(value) {
                    Ok(status) => out.push(status),
                    Err(err) => {
                        log::debug!("[journal] skipping undecodable run status key={key} err={err}")
                    }
                }
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl HarnessStatusStore for FileStatusStore {
    async fn put_status(&self, status: HarnessRunStatus) -> TaResult<()> {
        let key = status.run_id.as_str().to_string();
        let value = serde_json::to_value(&status)?;
        self.kv.put(STATUS_NS, &key, value).await
    }

    async fn get_status(&self, run_id: &str) -> TaResult<Option<HarnessRunStatus>> {
        match self.kv.get(STATUS_NS, run_id).await? {
            Some(value) => Ok(Some(serde_json::from_value(value)?)),
            None => Ok(None),
        }
    }

    async fn list_by_thread(&self, thread_id: &str) -> TaResult<Vec<HarnessRunStatus>> {
        Ok(self
            .all()
            .await?
            .into_iter()
            .filter(|s| {
                s.thread_id
                    .as_ref()
                    .is_some_and(|t| t.as_str() == thread_id)
            })
            .collect())
    }

    async fn list_by_root(&self, root_run_id: &str) -> TaResult<Vec<HarnessRunStatus>> {
        Ok(self
            .all()
            .await?
            .into_iter()
            .filter(|s| s.root_run_id.as_str() == root_run_id)
            .collect())
    }

    async fn list_active(&self) -> TaResult<Vec<HarnessRunStatus>> {
        use tinyagents_harness::ids::ExecutionStatus;
        Ok(self
            .all()
            .await?
            .into_iter()
            .filter(|s| {
                matches!(
                    s.status,
                    ExecutionStatus::Pending
                        | ExecutionStatus::Running
                        | ExecutionStatus::Interrupted
                )
            })
            .collect())
    }
}

/// A live handle to a turn's durable journal + status snapshot.
///
/// Held by the turn loop for the duration of the run so it can stamp a terminal
/// status (`completed` / `failed`) once the harness returns — the harness
/// `AgentEvent` stream carries no run-terminal event, so the authoritative
/// terminal write is caller-driven here. Every method is best-effort and
/// non-fatal.
pub(crate) struct TurnJournal {
    run_id: RunId,
    status_store: Arc<FileStatusStore>,
    /// The in-flight status snapshot, mutated in place to `completed`/`failed`.
    status: Mutex<HarnessRunStatus>,
}

impl TurnJournal {
    /// The durable run id — the journal stream key + status key a future replay
    /// RPC reads back via [`read_run_events`] / [`read_run_status`].
    // Part of the replay seam surfaced to the (follow-up) replay RPC.
    #[allow(dead_code)]
    pub(crate) fn run_id(&self) -> String {
        self.run_id.as_str().to_string()
    }

    /// Best-effort terminal write: mark the run completed and persist. Non-fatal.
    pub(crate) async fn finish_completed(&self) {
        let snapshot = {
            let mut guard = self.status.lock().unwrap();
            guard.mark_completed();
            guard.clone()
        };
        match self.status_store.put_status(snapshot).await {
            Ok(()) => log::debug!("[journal] run completed run_id={}", self.run_id.as_str()),
            Err(err) => log::debug!(
                "[journal] completed status write failed run_id={} err={err}",
                self.run_id.as_str()
            ),
        }
    }

    /// Best-effort terminal write: mark the run failed (recording `error`) and
    /// persist. Non-fatal.
    pub(crate) async fn finish_failed(&self, error: &str) {
        let snapshot = {
            let mut guard = self.status.lock().unwrap();
            guard.mark_failed(error);
            guard.clone()
        };
        match self.status_store.put_status(snapshot).await {
            Ok(()) => log::warn!(
                "[journal] run failed run_id={} error={error}",
                self.run_id.as_str()
            ),
            Err(err) => log::debug!(
                "[journal] failed status write failed run_id={} err={err}",
                self.run_id.as_str()
            ),
        }
    }
}

/// Attach a durable event journal + status writer to `events`, *in addition to*
/// the existing (untouched) [`OpenhumanEventBridge`] subscription.
///
/// `run_id` MUST be the same id the caller passed to
/// [`EventSink::with_stream_id`] when it created `events` (mint it once via
/// [`mint_run_id`]). That shared id is what makes the persisted `event_id`s the
/// restart-stable `{run_id}-evt-{offset}` a late-attach replay reconstructs the
/// timeline from. `thread_id` (when known — e.g. the sub-agent task scope)
/// records the run under a thread so [`FileStatusStore::list_by_thread`] answers.
///
/// Returns a [`TurnJournal`] handle the caller uses to stamp the terminal
/// status after the run, or `None` when the store could not be opened (the run
/// proceeds unaffected — journaling is best-effort). Safe to call for observed
/// and unobserved turns alike: it does not depend on `on_progress`.
///
/// [`OpenhumanEventBridge`]: crate::openhuman::agent::tinyagents::observability::OpenhumanEventBridge
/// [`EventSink::with_stream_id`]: tinyagents_harness::events::EventSink::with_stream_id
pub(crate) async fn attach_turn_journal(
    events: &EventSink,
    model: &str,
    run_id: RunId,
    thread_id: Option<ThreadId>,
) -> Option<TurnJournal> {
    let workspace = match resolve_workspace().await {
        Ok(dir) => dir,
        Err(err) => {
            log::debug!("[journal] skipping journal attach; {err}");
            return None;
        }
    };

    let stores = open_session_stores(&workspace);

    // Event journal: crate StoreEventJournal over the 04-sessions JsonlAppendStore
    // (stream key = run id). Wrapped in a JournalSink (stamps run lineage) and a
    // RedactingSink (masks process credentials) before persisting. Because
    // `events` was seeded with `with_stream_id(run_id)`, every persisted
    // observation's `event_id` is the stable `{run_id}-evt-{offset}`.
    let journal: Arc<dyn HarnessEventJournal> = Arc::new(StoreEventJournal::new(stores.journal));
    let journal_sink = JournalSink::new(journal, run_id.clone());
    let redacting = RedactingSink::new(Arc::new(journal_sink), openhuman_redaction_secrets());

    // FanOutSink is the durable-observer composition seam (05.2 adds graph sinks
    // here). Subscribing it as its own listener leaves the bridge subscription
    // untouched — the EventSink fans out to both.
    let fanout = FanOutSink::new().with(Arc::new(redacting));
    events.subscribe(Arc::new(fanout));

    // Status store: durable, Store-backed. Seed an initial `running` snapshot,
    // recording the thread (when known) so list_by_thread answers at run start.
    let status_store = Arc::new(FileStatusStore::new(stores.kv));
    let mut status = HarnessRunStatus::new(run_id.clone(), ComponentId::new(model.to_string()));
    if let Some(thread_id) = thread_id {
        status = status.with_thread(thread_id);
    }
    status.mark_running(HarnessPhase::Model);
    if let Err(err) = status_store.put_status(status.clone()).await {
        log::debug!(
            "[journal] initial status write failed run_id={} err={err}",
            run_id.as_str()
        );
    }

    log::debug!(
        "[journal] attached durable event journal run_id={} thread={:?} model={model}",
        run_id.as_str(),
        status.thread_id.as_ref().map(|t| t.as_str())
    );
    Some(TurnJournal {
        run_id,
        status_store,
        status: Mutex::new(status),
    })
}

/// Late-attach replay reader: return every persisted observation for `run_id`
/// whose stream offset is `>= from_offset`, in order. Reading from `0` replays
/// the whole run.
///
/// This is the seam a future replay RPC (05.x) will call so the desktop can
/// reconnect mid-run and backfill the timeline from durable state instead of
/// relying on transient `AgentProgress` buffering. Best-effort: a missing store
/// or unknown run yields an empty `Vec`, not an error.
// Replay-RPC seam (05.x): no in-tree caller yet — the persisted journal is
// provably replayable through this reader.
#[allow(dead_code)]
pub(crate) async fn read_run_events(
    run_id: &str,
    from_offset: u64,
) -> anyhow::Result<Vec<AgentObservation>> {
    let workspace = resolve_workspace().await?;
    let stores = open_session_stores(&workspace);
    let journal = StoreEventJournal::new(stores.journal);
    journal
        .read_from(run_id, from_offset)
        .await
        .map_err(|e| anyhow::anyhow!("[journal] read_run_events failed run_id={run_id}: {e}"))
}

/// Late-attach replay reader: the latest durable [`HarnessRunStatus`] for
/// `run_id`, or `None` if unknown. Companion to [`read_run_events`] for the
/// future replay RPC.
#[allow(dead_code)]
pub(crate) async fn read_run_status(run_id: &str) -> anyhow::Result<Option<HarnessRunStatus>> {
    let workspace = resolve_workspace().await?;
    let stores = open_session_stores(&workspace);
    let status = FileStatusStore::new(stores.kv);
    status
        .get_status(run_id)
        .await
        .map_err(|e| anyhow::anyhow!("[journal] read_run_status failed run_id={run_id}: {e}"))
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
