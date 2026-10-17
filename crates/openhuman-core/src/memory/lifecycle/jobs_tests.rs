use super::*;

use tinymemory_api::{ConsolidateRequest, ItemKind, Reach};

use crate::memory::test_fixtures::{bind_reference, config_in};

fn build(agent: &str) -> BackgroundJob {
    BackgroundJob::BuildBeliefs {
        request: ConsolidateRequest::new(Reach::exact(Namespace::agent(agent)))
            .kinds([ItemKind::Conversation]),
    }
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
    assert!(path(&config.workspace_dir).exists());
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
        Err(MemoryError::InvalidRequest(_))
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
        Err(MemoryError::Off(_))
    ));
    run_due(&config).await;
    assert_eq!(snapshot(&config).await.pending.len(), 1);
}

#[tokio::test]
async fn the_build_delay_comes_from_the_recall_config_and_the_file_store_is_the_workspaces() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    bind_reference(&config);
    config.memory.recall.build_delay_secs = 3600;
    enqueue(&config, &Namespace::ROOT, vec![build("a")]).await;
    assert!(has_pending(&config.workspace_dir));
    let on_disk: JobQueue =
        serde_json::from_str(&std::fs::read_to_string(path(&config.workspace_dir)).unwrap())
            .unwrap();
    assert_eq!(on_disk.pending.len(), 1, "persisted in memory/jobs.json");
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
