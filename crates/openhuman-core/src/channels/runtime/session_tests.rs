use super::*;
use crate::core::runtime::DispatchCtx;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

#[tokio::test]
async fn independent_local_runtime_does_not_need_an_account_session() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let _signed_out = crate::platform::llm_gate::SignedOutTestGuard::set(true);
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
    let _signed_out = crate::platform::llm_gate::SignedOutTestGuard::set(false);
    let old_session = channel_session();
    crate::security::credentials::ops::clear_session(&DispatchCtx::process_default(), &config)
        .await
        .unwrap();
    assert!(old_session.is_cancelled());
    assert!(!channel_session().is_cancelled());
}

#[tokio::test]
async fn channel_sign_in_resumes_listener_service_after_logout() {
    let controller = ChannelSessionController::new();
    let starts = Arc::new(AtomicUsize::new(0));
    let cleanups = Arc::new(AtomicUsize::new(0));
    let observed_starts = Arc::clone(&starts);
    let observed_cleanups = Arc::clone(&cleanups);
    let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
    let (cleaned_tx, mut cleaned_rx) = tokio::sync::mpsc::unbounded_channel();
    let service = spawn_channel_service_with(
        &DispatchCtx::unbound(),
        controller.clone(),
        move |session| {
            let starts = Arc::clone(&observed_starts);
            let cleanups = Arc::clone(&observed_cleanups);
            let started_tx = started_tx.clone();
            let cleaned_tx = cleaned_tx.clone();
            async move {
                let generation = starts.fetch_add(1, Ordering::SeqCst) + 1;
                let cleanups_before_start = cleanups.load(Ordering::SeqCst);
                let _ = started_tx.send((session.clone(), cleanups_before_start));
                session.cancelled().await;
                let completed_cleanup = cleanups.fetch_add(1, Ordering::SeqCst) + 1;
                let _ = cleaned_tx.send((generation, completed_cleanup));
                Ok(())
            }
        },
    );

    let (old, initial_cleanups) = started_rx.recv().await.expect("initial channel generation");
    assert_eq!(initial_cleanups, 0);
    controller.invalidate();
    controller.resume();
    controller.resume(); // Duplicate sign-in notifications must not fork a listener.

    let (current, cleanups_before_restart) =
        tokio::time::timeout(std::time::Duration::from_secs(1), started_rx.recv())
            .await
            .expect("sign-in wakes the stopped channel service")
            .expect("restarted channel generation");
    assert!(old.is_cancelled(), "logout must fence the old generation");
    assert!(
        !current.is_cancelled(),
        "sign-in resumes a fresh generation"
    );
    assert_eq!(
        cleanups_before_restart, 1,
        "old runtime cleanup must finish before restart"
    );
    assert_eq!(starts.load(Ordering::SeqCst), 2);
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(1), cleaned_rx.recv())
            .await
            .expect("first generation cleanup notification")
            .expect("cleanup notification")
            .0,
        1
    );

    controller.invalidate();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(1), cleaned_rx.recv())
            .await
            .expect("second generation cleanup completes")
            .expect("cleanup notification")
            .0,
        2
    );
    service.abort();
}

#[tokio::test]
async fn channel_service_created_during_logout_waits_for_sign_in() {
    let controller = ChannelSessionController::new();
    controller.invalidate();
    let starts = Arc::new(AtomicUsize::new(0));
    let observed_starts = Arc::clone(&starts);
    let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
    let service = spawn_channel_service_with(
        &DispatchCtx::unbound(),
        controller.clone(),
        move |session| {
            let starts = Arc::clone(&observed_starts);
            let started_tx = started_tx.clone();
            async move {
                starts.fetch_add(1, Ordering::SeqCst);
                let _ = started_tx.send(session.clone());
                session.cancelled().await;
                Ok(())
            }
        },
    );

    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), started_rx.recv())
            .await
            .is_err()
    );
    assert_eq!(starts.load(Ordering::SeqCst), 0);

    controller.resume();
    let resumed = tokio::time::timeout(std::time::Duration::from_secs(1), started_rx.recv())
        .await
        .expect("credential installation resumes startup")
        .expect("resumed channel generation");
    assert!(!resumed.is_cancelled());
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    controller.invalidate();
    service.abort();
}
