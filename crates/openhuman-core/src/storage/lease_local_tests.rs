use super::*;

#[tokio::test]
async fn doctor_holder_inspection_does_not_create_a_lock_file() {
    let dir = tempfile::tempdir().unwrap();
    let key_dir = local_dir(dir.path(), "k");
    fs::create_dir_all(&key_dir).unwrap();
    fs::write(
        key_dir.join(RECORD_FILE),
        serde_json::to_vec(&LeaseRecord {
            owner: "previous-node".into(),
            endpoint: None,
            epoch: 1,
            expires_at_ms: u64::MAX,
            released: false,
        })
        .unwrap(),
    )
    .unwrap();
    let lock_path = key_dir.join(LOCK_FILE);
    assert!(!lock_path.exists());

    let observer = LocalLeases::new(dir.path(), "doctor");
    let record = observer.holder_readonly("k").await.unwrap().unwrap();

    assert_eq!(record.owner, "previous-node");
    assert!(
        !record.is_live(0),
        "the absent lock means no holder is live"
    );
    assert!(
        !lock_path.exists(),
        "a read-only holder inspection must not create the lock file"
    );
}

#[tokio::test]
async fn a_second_instance_on_one_root_is_held_out() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (
        LocalLeases::new(dir.path(), "a"),
        LocalLeases::new(dir.path(), "b"),
    );
    let grant = a.acquire("user-1", 0).await.unwrap();
    assert_eq!((grant.epoch, grant.previous_unclean), (1, false));
    match b.acquire("user-1", 0).await {
        Err(LeaseError::Held(record)) => {
            assert_eq!(record.owner, "a");
            assert!(record.is_live(u64::MAX - 1));
        }
        other => panic!("expected Held, got {other:?}"),
    }
    assert_eq!(b.holder("user-1").await.unwrap().unwrap().owner, "a");
    assert!(b.holder("user-1").await.unwrap().unwrap().is_live(0));
}

#[tokio::test]
async fn release_hands_over_cleanly_and_stale_grants_are_lost() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (
        LocalLeases::new(dir.path(), "a"),
        LocalLeases::new(dir.path(), "b"),
    );
    let grant = a.acquire("k", 0).await.unwrap();
    let renewed = a.renew(&grant, 5).await.unwrap();
    assert!(matches!(a.renew(&grant, 6).await, Err(LeaseError::Lost)));
    let again = a.acquire("k", 7).await.unwrap();
    assert_eq!((again.epoch, again.previous_unclean), (1, false));
    a.release(renewed.clone()).await.unwrap();
    assert!(matches!(a.release(renewed).await, Err(LeaseError::Lost)));
    assert!(b.holder("k").await.unwrap().unwrap().released);
    let next = b.acquire("k", 8).await.unwrap();
    assert_eq!((next.epoch, next.previous_unclean), (2, false));
}

#[tokio::test]
async fn a_dead_holder_leaves_an_unclean_record() {
    let dir = tempfile::tempdir().unwrap();
    {
        let crashed = LocalLeases::new(dir.path(), "a");
        crashed.acquire("k", 0).await.unwrap();
        // Dropped without releasing: the lock goes with the file handle, as
        // it would with the process.
    }
    let b = LocalLeases::new(dir.path(), "b");
    let stale = b.holder("k").await.unwrap().unwrap();
    assert_eq!(stale.owner, "a");
    assert!(!stale.is_live(0), "a free lock means the holder is gone");
    let taken = b.acquire("k", 0).await.unwrap();
    assert_eq!((taken.epoch, taken.previous_unclean), (2, true));
}

#[tokio::test]
async fn keys_are_validated_and_nothing_is_held_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let a = LocalLeases::new(dir.path(), "a");
    assert!(matches!(
        a.acquire("../x", 0).await,
        Err(LeaseError::Storage(_))
    ));
    assert!(a.holder("free").await.unwrap().is_none());
    assert!(!dir.path().join("..").join("x").join(".lease").exists());
}

#[tokio::test]
async fn a_malformed_record_is_an_error_not_a_fresh_start() {
    let dir = tempfile::tempdir().unwrap();
    let a = LocalLeases::new(dir.path(), "a");
    let key_dir = local_dir(dir.path(), "k");
    fs::create_dir_all(&key_dir).unwrap();
    fs::write(key_dir.join(RECORD_FILE), b"{not json").unwrap();
    assert!(matches!(
        a.acquire("k", 0).await,
        Err(LeaseError::Storage(_))
    ));
    assert!(matches!(a.holder("k").await, Err(LeaseError::Storage(_))));
}

#[tokio::test]
async fn a_contended_key_with_a_malformed_record_is_a_storage_error() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (
        LocalLeases::new(dir.path(), "a"),
        LocalLeases::new(dir.path(), "b"),
    );
    a.acquire("k", 0).await.unwrap();
    std::fs::write(local_dir(dir.path(), "k").join(RECORD_FILE), b"{not json").unwrap();
    assert!(matches!(
        b.acquire("k", 1).await,
        Err(LeaseError::Storage(_))
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn a_failed_release_write_keeps_the_lock_and_the_holding() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (
        LocalLeases::new(dir.path(), "a"),
        LocalLeases::new(dir.path(), "b"),
    );
    let grant = a.acquire("k", 0).await.unwrap();
    let key_dir = local_dir(dir.path(), "k");
    std::fs::set_permissions(&key_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::write(key_dir.join("probe"), b"").is_ok() {
        // Running with privileges that ignore the mode (root): not testable.
        std::fs::set_permissions(&key_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let failed = a.release(grant.clone()).await;
    let contended = b.acquire("k", 1).await;
    std::fs::set_permissions(&key_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(failed.is_err());
    match contended {
        Err(LeaseError::Held(record)) => assert!(!record.released),
        other => panic!("expected Held, got {other:?}"),
    }
    a.release(grant).await.unwrap();
}
