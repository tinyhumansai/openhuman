use super::*;
use std::sync::Arc;

use tinyagents_harness::events::{AgentEvent, EventSink};
use tinyagents_harness::ids::{ComponentId, HarnessPhase, ThreadId};
use tinyagents_harness::observability::{FanOutSink, JournalSink, StoreEventJournal};

use crate::openhuman::agent::tinyagents::journal::mint_run_id;

/// Seed `count` durable events for a fresh run under `workspace`, returning
/// the run id. Mirrors the seam wiring in the `journal.rs` tests: a run
/// [`EventSink`] seeded with the run id (so persisted `event_id`s are the
/// stable `{run_id}-evt-{offset}`) fanning out into a [`StoreEventJournal`].
async fn seed_run_events(workspace: &Path, count: usize) -> String {
    let stores = open_session_stores(workspace);
    let run_id = mint_run_id();
    let journal: Arc<dyn HarnessEventJournal> = Arc::new(StoreEventJournal::new(stores.journal));
    let sink = EventSink::with_stream_id(run_id.as_str());
    let journal_sink = JournalSink::new(journal, run_id.clone());
    sink.subscribe(Arc::new(FanOutSink::new().with(Arc::new(journal_sink))));
    for i in 0..count {
        sink.emit(AgentEvent::ToolStarted {
            call_id: format!("c{i}").into(),
            tool_name: format!("tool-{i}"),
        });
    }
    run_id.as_str().to_string()
}

/// Paging boundary: a page smaller than the stream reports a `next_offset`
/// cursor; the final page drains to `None` and never over-reads.
#[tokio::test]
async fn read_run_events_page_pages_and_drains() {
    let tmp = std::env::temp_dir().join(format!("oh-replay-page-{}", uuid::Uuid::new_v4()));
    let run_id = seed_run_events(&tmp, 3).await;

    // First page (limit 2) returns offsets 0,1 with a cursor at offset 2.
    let page1 = read_run_events_page(&tmp, &run_id, 0, 2).await.unwrap();
    assert_eq!(page1.events.len(), 2);
    assert_eq!(page1.events[0].offset, 0);
    assert_eq!(page1.events[1].offset, 1);
    assert_eq!(page1.next_offset, Some(2), "more events remain");

    // Second page resumes at the cursor and drains → next_offset None.
    let page2 = read_run_events_page(&tmp, &run_id, page1.next_offset.unwrap(), 2)
        .await
        .unwrap();
    assert_eq!(page2.events.len(), 1);
    assert_eq!(page2.events[0].offset, 2);
    assert_eq!(page2.next_offset, None, "stream drained on the last page");

    // A page exactly the size of the remaining stream still drains to None
    // (no phantom extra page).
    let exact = read_run_events_page(&tmp, &run_id, 0, 3).await.unwrap();
    assert_eq!(exact.events.len(), 3);
    assert_eq!(exact.next_offset, None);

    // Unknown run → empty page, not an error.
    let empty = read_run_events_page(&tmp, "run.does-not-exist", 0, 10)
        .await
        .unwrap();
    assert!(empty.events.is_empty());
    assert_eq!(empty.next_offset, None);

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Status reader returns `None` for a run that was never recorded.
#[tokio::test]
async fn read_run_status_none_for_unknown_run() {
    let tmp = std::env::temp_dir().join(format!("oh-replay-status-{}", uuid::Uuid::new_v4()));
    let missing = read_run_status(&tmp, "run.nope").await.unwrap();
    assert!(missing.is_none());
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Active listing surfaces a started run and filters by thread; a completed
/// run is excluded.
#[tokio::test]
async fn list_active_runs_returns_started_and_filters_by_thread() {
    let tmp = std::env::temp_dir().join(format!("oh-replay-active-{}", uuid::Uuid::new_v4()));
    let store = FileStatusStore::new(open_session_stores(&tmp).kv);

    // A running run on thread-A.
    let run_a = mint_run_id();
    let mut status_a =
        HarnessRunStatus::new(run_a.clone(), ComponentId::new("mock-model".to_string()))
            .with_thread(ThreadId::new("thread-A"));
    status_a.mark_running(HarnessPhase::Model);
    store.put_status(status_a).await.unwrap();

    // A completed run on thread-B (must NOT appear in the active listing).
    let run_b = mint_run_id();
    let mut status_b =
        HarnessRunStatus::new(run_b.clone(), ComponentId::new("mock-model".to_string()))
            .with_thread(ThreadId::new("thread-B"));
    status_b.mark_running(HarnessPhase::Model);
    status_b.mark_completed();
    store.put_status(status_b).await.unwrap();

    // No filter: only the running run.
    let active = list_active_runs(&tmp, None, None).await.unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].run_id.as_str(), run_a.as_str());

    // Filter by thread-A: the running run is returned.
    let by_thread_a = list_active_runs(&tmp, Some("thread-A"), None)
        .await
        .unwrap();
    assert_eq!(by_thread_a.len(), 1);
    assert_eq!(by_thread_a[0].run_id.as_str(), run_a.as_str());

    // Filter by thread-B: the only run there is completed → excluded.
    let by_thread_b = list_active_runs(&tmp, Some("thread-B"), None)
        .await
        .unwrap();
    assert!(by_thread_b.is_empty());

    // Filter by an unknown thread: empty.
    let by_thread_none = list_active_runs(&tmp, Some("nope"), None).await.unwrap();
    assert!(by_thread_none.is_empty());

    // Filter by root_run_id (a top-level run's root equals its own id).
    let by_root = list_active_runs(&tmp, None, Some(run_a.as_str()))
        .await
        .unwrap();
    assert_eq!(by_root.len(), 1);
    assert_eq!(by_root[0].run_id.as_str(), run_a.as_str());

    let _ = std::fs::remove_dir_all(&tmp);
}
