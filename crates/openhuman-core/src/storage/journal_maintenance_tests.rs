#![cfg(feature = "storage-sqlite")]

use super::*;
use crate::storage::test_support::{sqlite_backend, SqliteBackend};
use std::sync::Arc;
use tinyagents_harness::events::{AgentEvent, EventSink, HarnessRunStatus};
use tinyagents_harness::ids::{ComponentId, HarnessPhase, RunId};
use tinyagents_harness::observability::replay::read_run_events_page;
use tinyagents_harness::observability::{AgentObservation, HarnessEventJournal, StoreEventJournal};
use tinyagents_harness::store::{AppendStore, DriverAppendStore};
use tinyagents_harness::HarnessStatusStore;

fn backend() -> SqliteBackend {
    sqlite_backend().expect("SQLite test backend")
}

fn journal(backend: &Arc<dyn StorageBackend>) -> StoreEventJournal<Arc<dyn AppendStore>> {
    journal_in_scope(backend, &Scope::local())
}

fn journal_in_scope(
    backend: &Arc<dyn StorageBackend>,
    scope: &Scope,
) -> StoreEventJournal<Arc<dyn AppendStore>> {
    let storage = backend.for_scope(scope).unwrap();
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
        .info(&format!("{}{run}", DriverAppendStore::JOURNAL_PREFIX))
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
    let fixture = backend();
    let backend = &fixture.backend;
    let journal = journal(backend);
    seed(&journal, "run.a", 7).await;
    let before = pages(&journal, "run.a").await;
    journal.seal_run("run.a").await.unwrap();
    assert!(sealed(backend, "run.a").await);
    assert_eq!(pages(&journal, "run.a").await, before);
}

#[tokio::test]
async fn backfill_seals_finished_runs_but_not_active_ones_and_records_itself() {
    let fixture = backend();
    let backend = &fixture.backend;
    let journal = journal(backend);
    seed(&journal, "run.done", 2).await;
    seed(&journal, "run.live", 2).await;
    mark_active(backend, "run.live").await;

    let report = maintain_scope(backend, &Scope::local(), 0, 1_000)
        .await
        .unwrap();
    assert_eq!(report.backfilled, 1);
    assert!(sealed(backend, "run.done").await);
    assert!(!sealed(backend, "run.live").await);

    // Recorded: a later pass does nothing, even for a run that has since ended.
    seed(&journal, "run.late", 1).await;
    let report = maintain_scope(backend, &Scope::local(), 0, 1_000)
        .await
        .unwrap();
    assert_eq!(report.backfilled, 0);
    assert!(!sealed(backend, "run.late").await);
    let storage = backend.for_scope(&Scope::local()).unwrap();
    let marker = storage
        .documents()
        .get(
            BACKFILL_COLLECTION,
            &crate::storage::schema_support::key_of("inactive-journal-backfill-v1").unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(marker.doc["version"], BACKFILL_VERSION);
}

#[tokio::test]
async fn maintenance_deletes_a_sealed_journal_after_the_configured_retention() {
    let fixture = backend();
    let backend = &fixture.backend;
    let journal = journal(backend);
    seed(&journal, "run.old", 2).await;
    journal.seal_run("run.old").await.unwrap();

    let storage = backend.for_scope(&Scope::local()).unwrap();
    let sealed_at = storage
        .streams()
        .info("harness.journal/run.old")
        .await
        .unwrap()
        .unwrap()
        .sealed_ms
        .unwrap();
    // Inside the window: kept.
    let early = maintain_scope(
        backend,
        &Scope::local(),
        DAY_MS,
        sealed_at + DAY_MS as i64 / 2,
    )
    .await
    .unwrap();
    assert_eq!(early.deleted, 0);
    // Past one day, the configured retention (not the schema's 30).
    let late = maintain_scope(
        backend,
        &Scope::local(),
        DAY_MS,
        sealed_at + DAY_MS as i64 + 10,
    )
    .await
    .unwrap();
    assert_eq!(late.deleted, 1);
    let storage = backend.for_scope(&Scope::local()).unwrap();
    assert!(storage
        .streams()
        .info(&format!("{}run.old", DriverAppendStore::JOURNAL_PREFIX))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn zero_retention_days_keeps_sealed_journals_forever() {
    let fixture = backend();
    let backend = &fixture.backend;
    let journal = journal(backend);
    seed(&journal, "run.keep", 1).await;
    journal.seal_run("run.keep").await.unwrap();
    let storage = backend.for_scope(&Scope::local()).unwrap();
    let sealed_at = storage
        .streams()
        .info("harness.journal/run.keep")
        .await
        .unwrap()
        .unwrap()
        .sealed_ms
        .unwrap();
    let summary = maintain_scope(backend, &Scope::local(), 0, sealed_at + 400 * DAY_MS as i64)
        .await
        .unwrap();
    assert_eq!(summary.deleted, 0);
    assert!(sealed(backend, "run.keep").await);
}

#[tokio::test]
async fn dormant_or_expired_profile_fences_receive_no_journal_maintenance_writes() {
    let profile_root = tempfile::tempdir().unwrap();
    let fixture = backend();
    let backend = Arc::clone(&fixture.backend);
    let profile_host = crate::profiles::ProfileHost::with_backend(
        crate::core::runtime::SaasConfig::new(profile_root.path()),
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::full(), None),
        Some(Arc::clone(&backend)),
    )
    .unwrap();
    let profile_id = crate::profiles::ProfileId::for_user(
        "journal-dormant",
        crate::profiles::ProfileIdMode::Raw,
    )
    .unwrap();
    profile_host.provision(&profile_id).await.unwrap();

    let profile_scope = crate::storage::scope_for_profile(profile_id.as_str());
    let profile_journal = journal_in_scope(&backend, &profile_scope);
    seed(&profile_journal, "run.foreign", 1).await;
    profile_journal.seal_run("run.foreign").await.unwrap();
    let stream = format!("{}run.foreign", DriverAppendStore::JOURNAL_PREFIX);
    let sealed_at = backend
        .for_scope(&profile_scope)
        .unwrap()
        .streams()
        .info(&stream)
        .await
        .unwrap()
        .unwrap()
        .sealed_ms
        .unwrap();
    let raw = backend.for_scope(&profile_scope).unwrap();
    let expired_at = sealed_at + DAY_MS as i64 + 1;

    // Provisioned in the registry, but never opened here: it may be leased by
    // another host, so it must not appear in this process's maintenance pass.
    let (handles, scopes) = profile_scope_snapshot(profile_host.hosted_profiles(), u64::MAX);
    assert!(handles.is_empty());
    assert!(scopes.is_empty());
    let untouched = maintain_scopes(&backend, scopes, DAY_MS, expired_at)
        .await
        .unwrap();
    assert_eq!(untouched.deleted, 0);

    // Even a previously captured profile handle stops authorizing writes
    // after its lease fence expires.
    let opened = profile_host.open(&profile_id).await.unwrap();
    opened.lease_fence().renewed(0);
    let (handles, scopes) = profile_scope_snapshot(vec![opened], u64::MAX);
    assert!(handles.is_empty());
    assert!(scopes.is_empty());
    let expired = maintain_scopes(&backend, scopes, DAY_MS, expired_at)
        .await
        .unwrap();
    assert_eq!(expired.deleted, 0);
    assert!(raw.streams().info(&stream).await.unwrap().is_some());
}

#[tokio::test]
async fn captured_profile_lease_expiry_between_snapshot_and_mutation_refuses_writes() {
    let tmp = tempfile::tempdir().unwrap();
    let fixture = backend();
    let backend = Arc::clone(&fixture.backend);
    let host = crate::profiles::ProfileHost::with_backend(
        crate::core::runtime::SaasConfig::new(tmp.path()),
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::full(), None),
        Some(Arc::clone(&backend)),
    )
    .unwrap();
    let id = crate::profiles::ProfileId::for_user(
        "journal-fence-race",
        crate::profiles::ProfileIdMode::Raw,
    )
    .unwrap();
    host.provision(&id).await.unwrap();
    let profile = host.open(&id).await.unwrap();
    let profile_backend = host.storage_backend_for(&profile).unwrap().unwrap();
    let scope = crate::storage::scope_for_profile(id.as_str());
    let profile_journal = journal_in_scope(&profile_backend, &scope);
    seed(&profile_journal, "run.expiring", 1).await;
    profile_journal.seal_run("run.expiring").await.unwrap();
    let stream = format!("{}run.expiring", DriverAppendStore::JOURNAL_PREFIX);
    let sealed_at = backend
        .for_scope(&scope)
        .unwrap()
        .streams()
        .info(&stream)
        .await
        .unwrap()
        .unwrap()
        .sealed_ms
        .unwrap();
    let raw = backend.for_scope(&scope).unwrap();

    // Capture the exact admitted fence and scope list first. Expiration after
    // that point must still prevent both conditional stream deletion and the
    // success marker write.
    let work = captured_profile_work(
        &host,
        vec![Arc::clone(&profile)],
        crate::storage::fence::system_now_ms(),
    )
    .unwrap()
    .into_iter()
    .filter(|item| item.scope == scope)
    .collect::<Vec<_>>();
    assert_eq!(work.len(), 1);
    profile.lease_fence().renewed(0);
    let summary = maintain_work_items(work, DAY_MS, sealed_at + DAY_MS as i64 + 1)
        .await
        .unwrap();
    assert_eq!(summary.deleted, 0);
    assert_eq!(summary.scopes, 0);

    assert!(raw.streams().info(&stream).await.unwrap().is_some());
    assert!(raw
        .documents()
        .get(
            BACKFILL_COLLECTION,
            &crate::storage::schema_support::key_of("inactive-journal-backfill-v1").unwrap(),
        )
        .await
        .unwrap()
        .is_none());
}

#[test]
fn journal_scope_inventory_excludes_cluster() {
    let (_profiles, scopes) = known_scopes().unwrap();
    assert!(!scopes.contains(&Scope::cluster()));
}
