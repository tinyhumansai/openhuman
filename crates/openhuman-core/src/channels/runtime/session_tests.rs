use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[tokio::test]
async fn independent_local_runtime_does_not_need_an_account_session() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let _signed_out = crate::cron::scheduler_gate::SignedOutTestGuard::set(true);
    run_in_session(CancellationToken::new(), async { Ok(()) })
        .await
        .unwrap();
}

#[tokio::test]
async fn invalidated_startup_never_polls_workspace_or_listeners() {
    let session = CancellationToken::new();
    session.cancel();
    let polled = AtomicBool::new(false);
    run_in_session(session.clone(), async {
        polled.store(true, Ordering::SeqCst);
        Ok(())
    })
    .await
    .unwrap();
    assert!(!polled.load(Ordering::SeqCst));
    // Exercise the production entry point, not only the generic select helper.
    crate::channels::start_channels_with_session(crate::config::Config::default(), session)
        .await
        .unwrap();
}

#[tokio::test]
async fn logout_drops_the_old_runtime_and_aborts_owned_workers() {
    let session = CancellationToken::new();
    let logout = session.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    struct Dropped(Option<tokio::sync::oneshot::Sender<()>>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            let _ = self.0.take().unwrap().send(());
        }
    }
    let touched_after_logout = Arc::new(AtomicBool::new(false));
    let touched = touched_after_logout.clone();
    let runtime = tokio::spawn(run_in_session(session, async move {
        let _worker = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
            let _dropped = Dropped(Some(dropped_tx));
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
            touched.store(true, Ordering::SeqCst);
        }));
        std::future::pending::<anyhow::Result<()>>().await
    }));
    started_rx.await.unwrap();
    logout.cancel();
    runtime.await.unwrap().unwrap();
    dropped_rx.await.unwrap();
    assert!(!touched_after_logout.load(Ordering::SeqCst));
}

#[tokio::test]
async fn clear_session_invalidates_old_channels_and_opens_a_fresh_generation() {
    // Logout resolves the signed-out workspace through HOME. Keep the real
    // user profile untouched and restore the process environment on panic too.
    let _env_guard = crate::config::TEST_ENV_LOCK.lock().await;
    struct RestoreHome(Option<std::ffi::OsString>);
    impl Drop for RestoreHome {
        fn drop(&mut self) {
            unsafe {
                match self.0.take() {
                    Some(value) => std::env::set_var("HOME", value),
                    None => std::env::remove_var("HOME"),
                }
            }
        }
    }
    let tmp = tempfile::TempDir::new().unwrap();
    let _home = RestoreHome(std::env::var_os("HOME"));
    unsafe { std::env::set_var("HOME", tmp.path()) };
    let config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    };
    let _signed_out = crate::cron::scheduler_gate::SignedOutTestGuard::set(false);
    let old_session = channel_session();
    crate::security::credentials::ops::clear_session(&config)
        .await
        .unwrap();
    assert!(old_session.is_cancelled());
    assert!(!channel_session().is_cancelled());
}
