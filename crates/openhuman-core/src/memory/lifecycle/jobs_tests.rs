use super::*;
use tinymemory_tools::queue::QueueState;
use tinymemory_tools::queue::MAX_ATTEMPTS;

use tinymemory_api::{ConsolidateRequest, ItemKind, Reach};

use crate::memory::test_fixtures::{bind_reference, config_in};

#[test]
fn workspace_queues_are_released_after_the_last_holder_drops() {
    let temp = tempfile::tempdir().unwrap();
    let workspaces = (0..32)
        .map(|index| temp.path().join(index.to_string()))
        .collect::<Vec<_>>();
    let queues = workspaces
        .iter()
        .map(|workspace| queue_of(workspace))
        .collect::<Vec<_>>();

    drop(queues);

    let sentinel = temp.path().join("sentinel");
    drop(queue_of(&sentinel));

    let queues = QUEUES.lock().unwrap();
    assert!(workspaces
        .iter()
        .all(|workspace| !queues.contains_key(workspace)));
}

fn build(agent: &str) -> BackgroundJob {
    BackgroundJob::BuildBeliefs {
        request: ConsolidateRequest::new(Reach::exact(Namespace::agent(agent)))
            .kinds([ItemKind::Conversation]),
    }
}

#[test]
fn corrupt_legacy_job_queue_is_skipped_without_replacing_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    let file = crate::memory::state_store::legacy_path(tmp.path(), "jobs");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"{\"pending\":[").unwrap();
    let store = FileQueueStore {
        workspace_dir: tmp.path().to_path_buf(),
    };

    assert_eq!(store.load(), QueueState::default());
    assert!(
        file.exists(),
        "the read-only legacy import must leave corrupt bytes in place"
    );
    assert_eq!(std::fs::read(file).unwrap(), b"{\"pending\":[");
}

#[tokio::test]
async fn duplicates_merge_and_the_queue_survives_a_reload() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    enqueue(
        &config,
        &Namespace::ROOT,
        vec![build("a"), build("a"), build("b")],
    )
    .await;
    enqueue(&config, &Namespace::ROOT, vec![build("a")]).await;
    let team: Namespace = "team:acme".parse().unwrap();
    enqueue(&config, &team, vec![build("a")]).await;

    let queue = snapshot(&config).await;
    assert_eq!(queue.pending.len(), 3, "one per (root, job)");
    assert!(
        crate::memory::state_store::load::<JobQueue>(&config.workspace_dir, "jobs")
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn due_jobs_wait_for_the_build_delay() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    bind_reference(&config);
    config.memory.recall.build_delay_secs = 3600;
    enqueue(&config, &Namespace::ROOT, vec![build("a")]).await;

    let runs = run(&config, Selection::Due).await.unwrap();
    assert!(runs.is_empty(), "a fresh job is not due yet");
    assert_eq!(snapshot(&config).await.pending.len(), 1);

    config.memory.recall.build_delay_secs = 0;
    let runs = run(&config, Selection::Due).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].outcome, "done",
        "the reference engine consolidates on demand"
    );
    let queue = snapshot(&config).await;
    assert!(queue.pending.is_empty());
    assert_eq!(queue.history.len(), 1);
}

#[tokio::test]
async fn one_job_runs_by_id_and_an_unknown_id_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);
    enqueue(&config, &Namespace::ROOT, vec![build("a"), build("b")]).await;
    let id = snapshot(&config).await.pending[1].id.clone();

    let runs = run(&config, Selection::One(id.clone())).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, id);
    assert_eq!(snapshot(&config).await.pending.len(), 1);
    assert!(matches!(
        run(&config, Selection::One("nope".into())).await,
        Err(Error::InvalidRequest(_))
    ));
    assert_eq!(
        snapshot(&config).await.pending.len(),
        1,
        "a refused run keeps the queue"
    );
}

#[tokio::test]
async fn a_run_needs_an_engine() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    enqueue(&config, &Namespace::ROOT, vec![build("a")]).await;
    assert!(matches!(
        run(&config, Selection::All).await,
        Err(Error::Off(_))
    ));
    run_due(&config).await;
    assert_eq!(snapshot(&config).await.pending.len(), 1);
}

#[tokio::test]
async fn the_build_delay_comes_from_recall_config_and_the_queue_uses_the_state_store() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    bind_reference(&config);
    config.memory.recall.build_delay_secs = 3600;
    enqueue(&config, &Namespace::ROOT, vec![build("a")]).await;
    assert!(has_pending(&config.workspace_dir));
    let stored: QueueState = crate::memory::state_store::load(&config.workspace_dir, "jobs")
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.pending.len(),
        1,
        "persisted through the state KV port"
    );
    assert!(run(&config, Selection::Due).await.unwrap().is_empty());
}

#[tokio::test]
async fn an_account_wide_refusal_through_the_host_keeps_the_job_without_an_attempt() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    crate::memory::test_fixtures::RefusingEngine::out_of_credits().bind(&config);
    enqueue(&config, &Namespace::ROOT, vec![build("a")]).await;
    for _ in 0..MAX_ATTEMPTS + 1 {
        assert_eq!(
            run(&config, Selection::All).await.unwrap()[0].outcome,
            "failed"
        );
    }
    let queue = snapshot(&config).await;
    assert_eq!(queue.pending.len(), 1);
    assert_eq!(queue.pending[0].attempts, 0);
}
