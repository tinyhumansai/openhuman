use super::*;

fn config_in(tmp: &tempfile::TempDir) -> Config {
    Config {
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().unwrap()
}

fn session(created_at: u64) -> Vec<u8> {
    serde_json::json!({
        "state": "legacy-state",
        "verifier": "legacy-verifier",
        "redirect_uri": "http://127.0.0.1:1455/auth/callback",
        "created_at": created_at,
    })
    .to_string()
    .into_bytes()
}

#[test]
fn the_fallback_store_is_shared_per_state_dir_and_not_across_dirs() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let first = store(&config_in(&a)).unwrap();
    let again = store(&config_in(&a)).unwrap();
    let other = store(&config_in(&b)).unwrap();

    runtime().block_on(async {
        first.set(PENDING_SECRET, b"one").await.unwrap();
        assert_eq!(
            again.get(PENDING_SECRET).await.unwrap().unwrap().as_slice(),
            b"one"
        );
        assert!(other.get(PENDING_SECRET).await.unwrap().is_none());
    });
}

#[test]
fn a_fresh_legacy_session_is_adopted_once_and_the_file_is_left_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let legacy = tmp.path().join(LEGACY_FILE);
    let bytes = session(now_secs());
    std::fs::write(&legacy, &bytes).unwrap();
    let secrets = store(&config_in(&tmp)).unwrap();

    runtime().block_on(async {
        adopt_legacy(tmp.path(), &secrets).await;
        assert_eq!(
            secrets
                .get(PENDING_SECRET)
                .await
                .unwrap()
                .unwrap()
                .as_slice(),
            bytes.as_slice()
        );

        // Consumed by a completed flow, it is not resurrected from the file.
        secrets.delete(PENDING_SECRET).await.unwrap();
        adopt_legacy(tmp.path(), &secrets).await;
        assert!(secrets.get(PENDING_SECRET).await.unwrap().is_none());
    });
    assert_eq!(std::fs::read(&legacy).unwrap(), bytes);
    forget_for_tests(tmp.path());
}

#[test]
fn an_expired_or_malformed_legacy_session_is_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    let secrets = store(&config_in(&tmp)).unwrap();

    runtime().block_on(async {
        std::fs::write(tmp.path().join(LEGACY_FILE), session(1)).unwrap();
        adopt_legacy(tmp.path(), &secrets).await;
        assert!(secrets.get(PENDING_SECRET).await.unwrap().is_none());

        forget_for_tests(tmp.path());
        let secrets = store(&config_in(&tmp)).unwrap();
        std::fs::write(tmp.path().join(LEGACY_FILE), b"not json").unwrap();
        adopt_legacy(tmp.path(), &secrets).await;
        assert!(secrets.get(PENDING_SECRET).await.unwrap().is_none());
    });
}

#[test]
fn a_session_already_in_the_store_wins_over_the_legacy_file() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join(LEGACY_FILE), session(now_secs())).unwrap();
    let secrets = store(&config_in(&tmp)).unwrap();

    runtime().block_on(async {
        secrets.set(PENDING_SECRET, b"newer").await.unwrap();
        adopt_legacy(tmp.path(), &secrets).await;
        assert_eq!(
            secrets
                .get(PENDING_SECRET)
                .await
                .unwrap()
                .unwrap()
                .as_slice(),
            b"newer"
        );
    });
}

#[test]
fn an_oversized_legacy_file_is_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join(LEGACY_FILE),
        vec![b' '; (MAX_LEGACY_BYTES + 1) as usize],
    )
    .unwrap();
    let secrets = store(&config_in(&tmp)).unwrap();

    runtime().block_on(async {
        adopt_legacy(tmp.path(), &secrets).await;
        assert!(secrets.get(PENDING_SECRET).await.unwrap().is_none());
    });
}
