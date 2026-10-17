use super::*;
use crate::agent::autonomous::types::Phase;

#[tokio::test]
async fn survives_reload_and_rejects_stale_generation() {
    let tmp = tempfile::tempdir().unwrap();
    let work = start(
        tmp.path(),
        "durable",
        "Ship feature".into(),
        vec!["tests pass".into()],
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        get(tmp.path(), "durable").await.unwrap().unwrap().id,
        work.id
    );
    assert_eq!(list(tmp.path()).await.unwrap().len(), 1);
    assert!(update(tmp.path(), "durable", "old-generation", |_| Ok(()))
        .await
        .is_err());
    assert!(start(
        tmp.path(),
        "durable",
        "Overwrite".into(),
        vec!["a".into()],
        None
    )
    .await
    .is_err());
    let goal = crate::agent::goals::store::get(tmp.path(), "durable")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(goal.objective, "Ship feature");
}

#[tokio::test]
async fn scoped_stop_does_not_stop_a_new_lease() {
    let tmp = tempfile::tempdir().unwrap();
    let work = start(
        tmp.path(),
        "stop",
        "Ship feature".into(),
        vec!["tests pass".into()],
        None,
    )
    .await
    .unwrap();
    update(tmp.path(), "stop", &work.id, |w| {
        w.lease = Some("new".into());
        Ok(())
    })
    .await
    .unwrap();
    pause(tmp.path(), "stop", Some("old")).await.unwrap();
    assert!(get(tmp.path(), "stop").await.unwrap().unwrap().runnable());
    pause(tmp.path(), "stop", Some("new")).await.unwrap();
    assert_eq!(
        get(tmp.path(), "stop").await.unwrap().unwrap().phase,
        Phase::Paused
    );
}

#[tokio::test]
async fn concurrent_starts_have_one_winner_and_one_matching_goal() {
    let tmp = tempfile::tempdir().unwrap();
    let (a, b) = tokio::join!(
        start(tmp.path(), "race", "A".into(), vec!["a".into()], None),
        start(tmp.path(), "race", "B".into(), vec!["b".into()], None)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    let work = get(tmp.path(), "race").await.unwrap().unwrap();
    let goal = crate::agent::goals::store::get(tmp.path(), "race")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(work.goal_id, goal.goal_id);
    assert_eq!(work.objective, goal.objective);
}
