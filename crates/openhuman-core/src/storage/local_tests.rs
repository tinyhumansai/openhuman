use super::*;
use crate::storage::documents::Repo;
use serde_json::json;
use tinystoragedrivers::{CollectionSpec, Precondition};

fn things() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new("things")]
}

fn put_thing(opened: &Opened, id: &'static str, n: i64) {
    let repo = Repo::on(&opened.backend, &opened.scope, "things", things).unwrap();
    repo.run(move |docs| async move {
        docs.put("things", id, json!({ "n": n }), Precondition::None)
            .await
            .map(|_| ())
    })
    .unwrap();
}

fn get_thing(opened: &Opened, id: &'static str) -> Option<i64> {
    let repo = Repo::on(&opened.backend, &opened.scope, "things", things).unwrap();
    repo.run(move |docs| async move { docs.get("things", id).await })
        .unwrap()
        .map(|stored| stored.doc["n"].as_i64().unwrap())
}

#[test]
// Ordinary unit test: the cfg(test) default must use the real driver testkit,
// not create a workspace database as a side effect.
fn an_uninstalled_call_opens_the_default_for_its_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let opened = open("things", dir.path()).unwrap();
    put_thing(&opened, "a", 1);
    assert!(
        !dir.path().join("storage").exists(),
        "ordinary unit tests use an in-memory testkit backend"
    );
    // The same workspace reuses the one backend.
    let again = open("things", dir.path()).unwrap();
    assert!(Arc::ptr_eq(&opened.backend, &again.backend));
    assert_eq!(get_thing(&again, "a"), Some(1));
}

#[test]
// Ordinary unit test: separate workspace defaults must remain isolated.
fn workspaces_do_not_share_a_default() {
    let one = tempfile::tempdir().unwrap();
    let two = tempfile::tempdir().unwrap();
    let a = open("things", one.path()).unwrap();
    let b = open("things", two.path()).unwrap();
    put_thing(&a, "only-in-one", 7);
    assert_eq!(get_thing(&b, "only-in-one"), None);
}

#[test]
#[cfg(feature = "storage-sqlite")]
// Persistence test: forgetting the process cache models a restart and must
// reopen the same durable workspace data.
fn forgetting_a_workspace_reopens_the_same_data() {
    let dir = tempfile::tempdir().unwrap();
    super::use_persistent_default_for_test(dir.path());
    let first = open("things", dir.path()).unwrap();
    put_thing(&first, "kept", 3);
    forget(dir.path());
    let second = open("things", dir.path()).unwrap();
    assert!(!Arc::ptr_eq(&first.backend, &second.backend));
    assert_eq!(
        get_thing(&second, "kept"),
        Some(3),
        "a restart keeps the data"
    );
}

#[tokio::test]
async fn concurrent_sync_and_async_open_share_one_workspace_backend() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let sync_workspace = workspace.clone();

    let (sync, asynchronous) = tokio::join!(
        tokio::task::spawn_blocking(move || backend_for(&sync_workspace)),
        backend_for_async(&workspace),
    );
    let sync = sync.unwrap().unwrap();
    let asynchronous = asynchronous.unwrap();

    assert!(
        Arc::ptr_eq(&sync, &asynchronous),
        "sync and async initializers must share the cached backend"
    );
    assert!(
        Arc::ptr_eq(&sync, &backend_for(dir.path()).unwrap()),
        "later sync opens must reuse the initialized backend"
    );
}

#[tokio::test]
async fn warm_sync_open_reuses_the_backend_without_entering_the_blocking_bridge() {
    let dir = tempfile::tempdir().unwrap();
    let warmed = backend_for_async(dir.path()).await.unwrap();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    super::arm_test_sync_backend_entry_signal(dir.path(), entered_tx);

    let synchronous = backend_for(dir.path()).unwrap();
    let bridge_entry = entered_rx.try_recv();
    super::clear_test_sync_backend_entry_signal(dir.path());

    assert!(Arc::ptr_eq(&warmed, &synchronous));
    assert_eq!(
        bridge_entry,
        Err(std::sync::mpsc::TryRecvError::Empty),
        "a warm backend lookup should not block the calling Tokio worker"
    );
}

#[test]
fn current_thread_sync_fallback_does_not_deadlock_on_async_backend_initialization() {
    const CHILD: &str = "OPENHUMAN_STORAGE_LOCAL_ASYNC_INIT_CHILD";
    const TEST: &str =
        "current_thread_sync_fallback_does_not_deadlock_on_async_backend_initialization";
    let test = format!("{}::{TEST}", module_path!().split_once("::").unwrap().1);

    if std::env::var(CHILD).as_deref() != Ok(test.as_str()) {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &test, "--nocapture", "--test-threads=1"])
            .env(CHILD, &test)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
        loop {
            if child.try_wait().unwrap().is_some() {
                let output = child.wait_with_output().unwrap();
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(
                    output.status.success() && stdout.contains("1 passed"),
                    "the bounded child must run and pass exactly one test; status={}\nstdout:\n{stdout}\nstderr:\n{stderr}",
                    output.status
                );
                return;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "current-thread storage initialization deadlocked; child exceeded 45s\nstdout:\n{}\nstderr:\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let workspace_for_async = dir.path().to_path_buf();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        super::install_test_backend_open_gate(started_tx, release_rx);

        let asynchronous =
            tokio::spawn(async move { backend_for_async(&workspace_for_async).await.unwrap() });
        started_rx
            .await
            .expect("async initialization claimed the workspace cell");

        let (sync_entry_tx, sync_entry_rx) = std::sync::mpsc::channel();
        super::arm_test_sync_backend_entry_signal(dir.path(), sync_entry_tx);
        let releaser = std::thread::spawn(move || {
            sync_entry_rx
                .recv_timeout(std::time::Duration::from_secs(30))
                .expect("the synchronous fallback reached backend_for");
            release_tx
                .send(())
                .expect("the gated backend initializer is still waiting");
        });

        // This intentionally blocks the current-thread runtime. The async
        // initializer must therefore be owned by the storage bridge instead
        // of waiting for a task queued on this same runtime.
        let synchronous = backend_for(dir.path()).unwrap();
        let asynchronous = asynchronous.await.unwrap();
        releaser.join().unwrap();
        assert!(Arc::ptr_eq(&synchronous, &asynchronous));
    });
}
