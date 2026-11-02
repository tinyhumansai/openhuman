use super::*;

fn round_trip_through_storage_bridge() -> serde_json::Value {
    let backend: Arc<dyn StorageBackend> =
        Arc::new(tinystoragedrivers_testkit::MemoryBackend::new());
    let storage = backend
        .for_scope(&Scope::new("bridge-test").unwrap())
        .unwrap();
    let docs = Arc::clone(storage.documents());
    block_on(async move {
        docs.ensure_collection(&CollectionSpec::new("bridge.records"))
            .await?;
        docs.put(
            "bridge.records",
            "record-1",
            serde_json::json!({"value": 42}),
            Precondition::Absent,
        )
        .await?;
        Ok(docs
            .get("bridge.records", "record-1")
            .await?
            .expect("record was written")
            .doc)
    })
    .unwrap()
}

fn config_with(url: Option<&str>) -> Config {
    let mut config = Config::default();
    config.storage.url = url.map(str::to_string);
    config
}

#[test]
fn the_environment_wins_over_the_config_file() {
    let config = config_with(Some("sqlite:/from/config"));
    assert_eq!(
        url_from(Some("memory".into()), &config).as_deref(),
        Some("memory")
    );
    assert_eq!(
        url_from(None, &config).as_deref(),
        Some("sqlite:/from/config")
    );
}

#[test]
fn blank_values_fall_through_to_the_workspace_default() {
    let config = config_with(None);
    let default = config::default_url(&config.workspace_dir);
    assert_eq!(url_from(None, &config).as_deref(), Some(default.as_str()));
    assert_eq!(
        url_from(Some("  ".into()), &config_with(Some(""))).as_deref(),
        Some(default.as_str())
    );
    assert_eq!(
        url_from(Some(" ".into()), &config_with(Some(" memory "))).as_deref(),
        Some("memory"),
        "a blank override falls through to the file"
    );
}

#[test]
fn the_default_url_is_a_directory_under_the_workspace() {
    let config = config_with(None);
    let url = url_from(None, &config).unwrap();
    assert_eq!(
        url,
        format!("sqlite:{}", config.workspace_dir.join("storage").display())
    );
    assert_eq!(explicit_url(&config), None, "nothing was configured");
    assert_eq!(
        explicit_url(&config_with(Some("memory"))).as_deref(),
        Some("memory")
    );
}

#[tokio::test]
async fn opens_memory_and_refuses_garbage() {
    let backend = open("memory").await.unwrap();
    let scope = backend.for_scope(&Scope::local()).unwrap();
    let docs = Arc::clone(scope.documents());
    block_on(async move {
        docs.ensure_collection(&CollectionSpec::new("probe"))
            .await?;
        docs.put(
            "probe",
            "opened",
            serde_json::json!({"ok": true}),
            Precondition::Absent,
        )
        .await?;
        assert_eq!(docs.get("probe", "opened").await?.unwrap().doc["ok"], true);
        Ok(())
    })
    .unwrap();
    assert!(open("ftp://nope").await.is_err());
}

#[test]
fn install_installed_and_clear() {
    // A private slot: installing into the process's would reroute every other
    // test's storage calls while this one runs.
    let slot = Slot::default();
    assert!(slot.installed().is_none());
    let backend = test_support::memory_backend();
    assert!(slot.install(Arc::clone(&backend)).is_none());
    assert!(slot
        .installed()
        .is_some_and(|got| Arc::ptr_eq(&got, &backend)));
    assert!(slot.install(test_support::memory_backend()).is_some());
    assert!(slot.clear());
    assert!(!slot.clear());
}

#[test]
fn agent_scopes_sit_under_the_local_tenant() {
    assert_eq!(scope_for_agent("agent-7").as_str(), "local/agent:agent-7");
    assert!(scope_for_agent("has a space")
        .as_str()
        .starts_with("local/agent:~"));
}

#[test]
fn the_scope_follows_the_acting_agent_and_fails_closed_in_saas() {
    assert_eq!(
        scope_from(None, Some("agent-7"), false).unwrap().as_str(),
        "local/agent:agent-7"
    );
    assert!(
        scope_from(None, Some("agent-7"), true).is_err(),
        "SaaS hands out profile scopes only"
    );
    assert!(scope_from(None, None, false).unwrap().is_local());
    assert!(
        scope_from(None, None, true).is_err(),
        "SaaS never falls back to a shared scope"
    );
}

#[test]
fn a_profile_scopes_storage_over_its_agents() {
    for saas in [false, true] {
        assert_eq!(
            scope_from(Some("u-alice"), Some("u-alice"), saas)
                .unwrap()
                .as_str(),
            "u:u-alice"
        );
        assert_eq!(
            scope_from(Some("u-alice"), None, saas).unwrap().as_str(),
            "u:u-alice"
        );
    }
    assert_ne!(scope_for_profile("a"), scope_for_profile("b"));
    let odd = scope_for_profile("has a space");
    assert!(odd.as_str().starts_with("u:~"), "{odd}");
    assert_ne!(odd, scope_for_profile("has  a space"));
    assert!(scope_for_profile(&"x".repeat(400))
        .as_str()
        .starts_with("u:~"));
}

#[test]
fn block_on_runs_a_future_from_sync_code() {
    let answer = block_on(async { Ok::<_, StorageError>(42) }).unwrap();
    assert_eq!(answer, 42);
    let error = block_on(async { Err::<(), _>(StorageError::conflict("raced")) }).unwrap_err();
    assert_eq!(error.message(), "raced");
}

#[tokio::test(flavor = "current_thread")]
async fn block_on_works_inside_a_current_thread_runtime() {
    assert_eq!(
        round_trip_through_storage_bridge(),
        serde_json::json!({"value": 42})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn block_on_works_inside_a_multi_thread_runtime() {
    assert_eq!(
        round_trip_through_storage_bridge(),
        serde_json::json!({"value": 42})
    );
}

#[tokio::test(flavor = "current_thread")]
async fn bridge_runtime_keeps_background_tasks_running_between_sync_calls() {
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let (progress_tx, progress_rx) = std::sync::mpsc::channel();
    block_on(async move {
        tokio::spawn(async move {
            let _ = ready_tx.send(());
            if release_rx.await.is_ok() {
                let _ = progress_tx.send(());
            }
        });
        ready_rx
            .await
            .map_err(|error| StorageError::backend(error.to_string()))?;
        Ok(())
    })
    .unwrap();

    // The task was spawned on the bridge runtime and the request has returned.
    // It must still make progress while the bridge owner waits for its next
    // synchronous request.
    release_tx.send(()).unwrap();
    progress_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("bridge runtime polled its background task while idle");
}

#[test]
fn only_mongodb_is_shared_between_processes() {
    assert!(driver_is_shared("mongodb"));
    for driver in ["sqlite", "memory", "file"] {
        assert!(!driver_is_shared(driver), "{driver}");
    }
}

#[test]
fn only_mongodb_and_sqlite_have_cross_process_cas() {
    for driver in ["mongodb", "sqlite"] {
        assert!(driver_has_cross_process_cas(driver), "{driver}");
    }
    for driver in ["memory", "file", "", "postgres"] {
        assert!(!driver_has_cross_process_cas(driver), "{driver}");
    }
}

#[tokio::test]
#[cfg(all(feature = "storage-sqlite", feature = "storage-file"))]
async fn local_drivers_do_not_claim_cross_process_cas() {
    let dir = tempfile::tempdir().unwrap();
    for (url, expected) in [
        ("memory".to_string(), false),
        (
            format!("sqlite:{}", dir.path().join("cas.db").display()),
            true,
        ),
        (
            format!("file:{}", dir.path().join("files").display()),
            false,
        ),
    ] {
        let backend = open(&url).await.unwrap();
        assert!(
            backend_has_cross_process_cas(backend.as_ref()) == expected,
            "unexpected CAS capability for {url}"
        );
    }
}

mod storage_runtime {
    use super::super::{block_on, connect_on_storage_runtime};
    use super::StorageError;
    use std::sync::mpsc as std_mpsc;
    use std::time::Duration;
    use tokio::sync::{mpsc, oneshot};

    #[derive(Clone)]
    struct FakePool {
        requests: mpsc::UnboundedSender<oneshot::Sender<u32>>,
    }

    impl FakePool {
        fn spawn() -> Self {
            let (requests, mut receiver) = mpsc::unbounded_channel::<oneshot::Sender<u32>>();
            tokio::spawn(async move {
                while let Some(reply) = receiver.recv().await {
                    let _ = reply.send(7);
                }
            });
            Self { requests }
        }

        async fn ping(&self) -> Result<u32, StorageError> {
            let (reply, answer) = oneshot::channel();
            self.requests
                .send(reply)
                .map_err(|_| StorageError::backend("fake pool stopped"))?;
            answer
                .await
                .map_err(|_| StorageError::backend("fake pool dropped the request"))
        }
    }

    #[tokio::test]
    async fn returns_what_connect_built() {
        let value = connect_on_storage_runtime(|| async { 41 + 1 })
            .await
            .expect("connect runs");
        assert_eq!(value, 42);
    }

    #[test]
    fn runs_from_inside_the_blocking_bridge() {
        let value = block_on(async {
            let pool = connect_on_storage_runtime(|| async { FakePool::spawn() }).await?;
            pool.ping().await
        })
        .expect("bridged connect completes");
        assert_eq!(value, 7);
    }

    #[tokio::test]
    async fn a_panicking_connect_is_a_storage_error() {
        let result = connect_on_storage_runtime(|| async {
            if true {
                panic!("connect blew up");
            }
        })
        .await;
        assert!(result.is_err(), "a panic surfaces as a StorageError");
    }

    #[derive(Clone, Default)]
    struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
        type Writer = Captured;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[tokio::test]
    async fn open_connects_on_the_storage_runtime() {
        let captured = Captured::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(captured.clone())
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        let backend = super::super::open("memory").await;
        drop(guard);
        assert!(backend.is_ok(), "memory backend opens");
        let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        assert!(
            log.contains("connecting on the storage runtime"),
            "open must connect through the storage runtime, got: {log}"
        );
    }

    #[test]
    fn bridged_calls_do_not_starve_a_capped_host_runtime() {
        let (done, outcome) = std_mpsc::channel();
        let host = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .max_blocking_threads(1)
                .enable_all()
                .build()
                .expect("host runtime");
            let result = runtime.block_on(async {
                let pool = connect_on_storage_runtime(|| async { FakePool::spawn() })
                    .await
                    .expect("connect");
                let calls: Vec<_> = (0..8)
                    .map(|_| {
                        let pool = pool.clone();
                        tokio::spawn(async move { block_on(async move { pool.ping().await }) })
                    })
                    .collect();
                let mut answers = Vec::new();
                for call in calls {
                    answers.push(call.await.expect("call task").expect("ping"));
                }
                answers
            });
            let _ = done.send(result);
            runtime
        });
        match outcome.recv_timeout(Duration::from_secs(20)) {
            Ok(answers) => {
                assert_eq!(answers, vec![7; 8]);
                drop(host.join().expect("host thread"));
            }
            Err(_) => {
                std::mem::forget(host);
                panic!("storage bridge deadlocked");
            }
        }
    }
}

/// A backend one test installs is not another concurrent test's storage: the
/// slot is visible to its installer's thread only (MAIN-57).
#[test]
fn a_test_installed_slot_is_hidden_from_other_test_threads() {
    let ready = std::sync::Arc::new(std::sync::Barrier::new(2));
    let done = std::sync::Arc::new(std::sync::Barrier::new(2));
    let installer = {
        let (ready, done) = (ready.clone(), done.clone());
        std::thread::Builder::new()
            .name("storage::slot_probe::installer".into())
            .spawn(move || {
                let _guard = crate::storage::test_support::install_backend_for_test(
                    crate::storage::test_support::memory_backend(),
                );
                let mine = crate::storage::installed().is_some();
                ready.wait();
                done.wait();
                assert!(mine, "the installer sees its own slot");
            })
            .unwrap()
    };
    let other = std::thread::Builder::new()
        .name("storage::slot_probe::other".into())
        .spawn(move || {
            ready.wait();
            let seen = installed_url().is_some() || crate::storage::installed().is_some();
            done.wait();
            seen
        })
        .unwrap();
    installer.join().unwrap();
    assert!(
        !other.join().unwrap(),
        "another test's thread must not see the installer's backend"
    );
}
