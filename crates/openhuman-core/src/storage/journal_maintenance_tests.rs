use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use tinyagents_harness::events::{AgentEvent, EventSink, HarnessRunStatus};
use tinyagents_harness::ids::{ComponentId, HarnessPhase, RunId};
use tinyagents_harness::observability::replay::read_run_events_page;
use tinyagents_harness::observability::{AgentObservation, HarnessEventJournal, StoreEventJournal};
use tinyagents_harness::store::{AppendStore, DriverAppendStore};
use tinyagents_harness::HarnessStatusStore;
use tinystoragedrivers::MemoryStorage;

fn backend_with_clock() -> (Arc<AtomicU64>, Arc<dyn StorageBackend>) {
    let now = Arc::new(AtomicU64::new(1_000));
    let clock = Arc::clone(&now);
    let backend: Arc<dyn StorageBackend> =
        Arc::new(MemoryStorage::with_clock(Arc::new(move || {
            clock.load(Ordering::SeqCst)
        })));
    (now, backend)
}

fn journal(backend: &Arc<dyn StorageBackend>) -> StoreEventJournal<Arc<dyn AppendStore>> {
    let storage = backend.for_scope(&Scope::local()).unwrap();
    StoreEventJournal::new(
        Arc::new(DriverAppendStore::journal(Arc::clone(storage.streams()))) as Arc<dyn AppendStore>,
    )
}

async fn seed(journal: &dyn HarnessEventJournal, run: &str, count: usize) {
    let sink = EventSink::with_stream_id(run);
    for i in 0..count {
        let record = sink.emit(AgentEvent::ToolStarted {
            parent_call_id: None,
            call_id: format!("c{i}").into(),
            tool_name: format!("tool-{i}"),
            input: None,
        });
        let obs = AgentObservation::from_record(&record, RunId::new(run), None, RunId::new(run));
        journal.append(obs).await.unwrap();
    }
}

async fn sealed(backend: &Arc<dyn StorageBackend>, run: &str) -> bool {
    backend
        .for_scope(&Scope::local())
        .unwrap()
        .streams()
        .info(&format!("{JOURNAL_STREAM}/{run}"))
        .await
        .unwrap()
        .unwrap()
        .sealed_ms
        .is_some()
}

async fn mark_active(backend: &Arc<dyn StorageBackend>, run: &str) {
    let storage = backend.for_scope(&Scope::local()).unwrap();
    let store = DriverStatusStore::new(Arc::clone(storage.documents()));
    let mut status = HarnessRunStatus::new(RunId::new(run), ComponentId::new("m".to_string()));
    status.mark_running(HarnessPhase::Model);
    store.put_status(status).await.unwrap();
}

async fn pages(
    journal: &dyn HarnessEventJournal,
    run: &str,
) -> Vec<(Vec<AgentObservation>, Option<u64>)> {
    let (mut out, mut cursor) = (Vec::new(), 0);
    loop {
        let page = read_run_events_page(journal, run, cursor, 3).await.unwrap();
        out.push((page.events, page.next_offset));
        match out.last().unwrap().1 {
            Some(next) => cursor = next,
            None => return out,
        }
    }
}

#[tokio::test]
async fn a_sealed_journal_serves_the_same_pages_as_an_unsealed_one() {
    let (_, backend) = backend_with_clock();
    let journal = journal(&backend);
    seed(&journal, "run.a", 7).await;
    let before = pages(&journal, "run.a").await;
    journal.seal_run("run.a").await.unwrap();
    assert!(sealed(&backend, "run.a").await);
    assert_eq!(pages(&journal, "run.a").await, before);
}

#[tokio::test]
async fn backfill_seals_finished_runs_but_not_active_ones_and_records_itself() {
    let (_, backend) = backend_with_clock();
    let journal = journal(&backend);
    seed(&journal, "run.done", 2).await;
    seed(&journal, "run.live", 2).await;
    mark_active(&backend, "run.live").await;

    assert_eq!(
        backfill_scope(backend.as_ref(), &Scope::local())
            .await
            .unwrap(),
        1
    );
    assert!(sealed(&backend, "run.done").await);
    assert!(!sealed(&backend, "run.live").await);

    // Recorded: a later pass does nothing, even for a run that has since ended.
    seed(&journal, "run.late", 1).await;
    assert_eq!(
        backfill_scope(backend.as_ref(), &Scope::local())
            .await
            .unwrap(),
        0
    );
    assert!(!sealed(&backend, "run.late").await);
    let storage = backend.for_scope(&Scope::local()).unwrap();
    assert!(Migrations::new(&storage)
        .done(BACKFILL_OWNER, BACKFILL_STEP)
        .await
        .unwrap());
}

#[tokio::test]
async fn maintenance_deletes_a_sealed_journal_after_the_configured_retention() {
    let (now, backend) = backend_with_clock();
    let journal = journal(&backend);
    seed(&journal, "run.old", 2).await;
    journal.seal_run("run.old").await.unwrap();

    // Inside the window: kept.
    let early = maintain_once(&backend, 1, 1_000 + DAY_MS as i64 / 2)
        .await
        .unwrap();
    assert_eq!(early.deleted, 0);
    // Past one day, the configured retention (not the schema's 30).
    now.store(1_000 + DAY_MS + 10, Ordering::SeqCst);
    let late = maintain_once(&backend, 1, 1_000 + DAY_MS as i64 + 10)
        .await
        .unwrap();
    assert_eq!(late.deleted, 1);
    let storage = backend.for_scope(&Scope::local()).unwrap();
    assert!(storage
        .streams()
        .info(&format!("{JOURNAL_STREAM}/run.old"))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn zero_retention_days_keeps_sealed_journals_forever() {
    let (_, backend) = backend_with_clock();
    let journal = journal(&backend);
    seed(&journal, "run.keep", 1).await;
    journal.seal_run("run.keep").await.unwrap();
    let summary = maintain_once(&backend, 0, 1_000 + 400 * DAY_MS as i64)
        .await
        .unwrap();
    assert_eq!(summary.deleted, 0);
    assert!(sealed(&backend, "run.keep").await);
}
