//! Durable event journals + status stores for tinyagents turns (issue #4249,
//! Workstream 05-events, 05.1).
//!
//! The live [`crate::openhuman::tinyagents::observability::OpenhumanEventBridge`]
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
//! ## Follow-ups (not in this slice)
//!
//! - A replay RPC (`agent.run_events`?) that surfaces [`read_run_events`] /
//!   [`read_run_status`] to the desktop for mid-run reconnect (05.x).
//! - Sub-agent / graph run lineage (`parent_run_id` / `root_run_id` threading)
//!   and per-thread status (`thread_id`) — wired in 05.2/05.3.
//! - Seeding the run [`EventSink`] with `with_stream_id(run_id)` for
//!   restart-stable event ids.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use tinyagents::error::Result as TaResult;
use tinyagents::harness::events::{EventSink, HarnessRunStatus};
use tinyagents::harness::ids::{ComponentId, HarnessPhase, RunId};
use tinyagents::harness::observability::{
    AgentObservation, FanOutSink, HarnessEventJournal, HarnessStatusStore, JournalSink,
    RedactingSink, StoreEventJournal,
};
use tinyagents::harness::store::{FileStore, Store};

use crate::openhuman::session_import::ops::open_session_stores;

/// KV namespace the durable per-run [`HarnessRunStatus`] snapshots live under
/// (`{workspace}/tinyagents_store/kv/run_status/<run_id>.json`). Slash-free so
/// it round-trips the crate [`FileStore`] name sanitizer.
const STATUS_NS: &str = "run_status";

/// Mints a fresh, slash-free, process-unique run id (`run.<32-hex>`), used both
/// as the journal stream key and the status-store key. The `simple()` uuid form
/// (no hyphens) keeps the id inside the crate store's allowed-character set.
fn new_run_id() -> RunId {
    RunId::new(format!("run.{}", uuid::Uuid::new_v4().simple()))
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
        use tinyagents::harness::ids::ExecutionStatus;
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
/// Returns a [`TurnJournal`] handle the caller uses to stamp the terminal
/// status after the run, or `None` when the store could not be opened (the run
/// proceeds unaffected — journaling is best-effort). Safe to call for observed
/// and unobserved turns alike: it does not depend on `on_progress`.
///
/// [`OpenhumanEventBridge`]: crate::openhuman::tinyagents::observability::OpenhumanEventBridge
pub(crate) async fn attach_turn_journal(events: &EventSink, model: &str) -> Option<TurnJournal> {
    let workspace = match resolve_workspace().await {
        Ok(dir) => dir,
        Err(err) => {
            log::debug!("[journal] skipping journal attach; {err}");
            return None;
        }
    };

    let stores = open_session_stores(&workspace);
    let run_id = new_run_id();

    // Event journal: crate StoreEventJournal over the 04-sessions JsonlAppendStore
    // (stream key = run id). Wrapped in a JournalSink (stamps run lineage) and a
    // RedactingSink (masks process credentials) before persisting.
    let journal: Arc<dyn HarnessEventJournal> = Arc::new(StoreEventJournal::new(stores.journal));
    let journal_sink = JournalSink::new(journal, run_id.clone());
    let redacting = RedactingSink::new(Arc::new(journal_sink), openhuman_redaction_secrets());

    // FanOutSink is the durable-observer composition seam (05.2 adds graph sinks
    // here). Subscribing it as its own listener leaves the bridge subscription
    // untouched — the EventSink fans out to both.
    let fanout = FanOutSink::new().with(Arc::new(redacting));
    events.subscribe(Arc::new(fanout));

    // Status store: durable, Store-backed. Seed an initial `running` snapshot.
    let status_store = Arc::new(FileStatusStore::new(stores.kv));
    let mut status = HarnessRunStatus::new(run_id.clone(), ComponentId::new(model.to_string()));
    status.mark_running(HarnessPhase::Model);
    if let Err(err) = status_store.put_status(status.clone()).await {
        log::debug!(
            "[journal] initial status write failed run_id={} err={err}",
            run_id.as_str()
        );
    }

    log::debug!(
        "[journal] attached durable event journal run_id={} model={model}",
        run_id.as_str()
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
mod tests {
    use super::*;
    use tinyagents::harness::events::AgentEvent;
    use tinyagents::harness::ids::ExecutionStatus;

    /// The full 05.1 acceptance in miniature: emit events through a sink with a
    /// durable journal attached, then reconstruct the timeline + terminal status
    /// from the store alone — the "kill the UI mid-turn, reattach, replay" path.
    #[tokio::test]
    async fn journal_persists_and_replays_run() {
        let tmp = std::env::temp_dir().join(format!("oh-journal-test-{}", uuid::Uuid::new_v4()));
        let stores = open_session_stores(&tmp);
        let run_id = new_run_id();

        // Attach a journal sink directly (bypassing config resolution) and emit.
        let journal: Arc<dyn HarnessEventJournal> =
            Arc::new(StoreEventJournal::new(stores.journal));
        let sink = EventSink::new();
        let journal_sink = JournalSink::new(journal, run_id.clone());
        let redacting = RedactingSink::new(Arc::new(journal_sink), vec!["sk-super-secret".into()]);
        sink.subscribe(Arc::new(FanOutSink::new().with(Arc::new(redacting))));

        sink.emit(AgentEvent::ModelStarted {
            call_id: "c1".into(),
            model: "sk-super-secret leaked here".to_string(),
        });
        sink.emit(AgentEvent::ToolStarted {
            call_id: "c1".into(),
            tool_name: "echo".to_string(),
        });

        // Reconstruct from the durable store alone.
        let replayed = read_run_events_at(&tmp, run_id.as_str(), 0).await;
        assert_eq!(replayed.len(), 2);
        // The seeded secret was masked before persistence.
        if let AgentEvent::ModelStarted { model, .. } = &replayed[0].event {
            assert!(
                !model.contains("sk-super-secret"),
                "secret should be redacted"
            );
            assert!(model.contains("[REDACTED]"));
        } else {
            panic!("expected ModelStarted first");
        }

        // Status store round-trips a running → completed transition + list_by_root.
        let status_store = FileStatusStore::new(open_session_stores(&tmp).kv);
        let mut status =
            HarnessRunStatus::new(run_id.clone(), ComponentId::new("mock-model".to_string()));
        status.mark_running(HarnessPhase::Model);
        status_store.put_status(status.clone()).await.unwrap();
        let active = status_store.list_active().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].status, ExecutionStatus::Running);

        status.mark_completed();
        status_store.put_status(status).await.unwrap();
        let by_root = status_store.list_by_root(run_id.as_str()).await.unwrap();
        assert_eq!(by_root.len(), 1);
        assert_eq!(by_root[0].status, ExecutionStatus::Completed);
        assert!(status_store.list_active().await.unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Workspace-parameterized twin of [`read_run_events`] for tests that supply
    /// an explicit store root instead of resolving one from config.
    async fn read_run_events_at(
        workspace: &std::path::Path,
        run_id: &str,
        from_offset: u64,
    ) -> Vec<AgentObservation> {
        let stores = open_session_stores(workspace);
        StoreEventJournal::new(stores.journal)
            .read_from(run_id, from_offset)
            .await
            .unwrap()
    }
}
