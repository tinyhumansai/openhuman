use super::*;

#[test]
fn core_shutdown_stops_and_joins_always_on_processing() {
    const CHILD_ENV: &str = "OPENHUMAN_ALWAYS_ON_SHUTDOWN_FIXTURE";
    if std::env::var(CHILD_ENV).as_deref() == Ok("1") {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(shutdown_fixture());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "voice::always_on::processor::tests::core_shutdown_stops_and_joins_always_on_processing",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    assert!(
        output.status.success(),
        "shutdown fixture failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

// A child process isolates the process-wide capture and processor state.
async fn shutdown_fixture() {
    ENABLED.store(true, Ordering::SeqCst);
    let mut stopped = super::super::capture::stop_subscription();
    let joined = std::sync::Arc::new(AtomicBool::new(false));
    let completed = joined.clone();
    *PROCESSOR.lock().unwrap() = Some(tokio::spawn(async move {
        stopped.changed().await.unwrap();
        completed.store(true, Ordering::SeqCst);
    }));

    crate::voice::capture_lease::shutdown_capture().await;

    assert!(
        !ENABLED.load(Ordering::SeqCst),
        "shutdown must disable processing"
    );
    assert!(
        joined.load(Ordering::SeqCst),
        "shutdown must join the processor"
    );
    assert!(PROCESSOR.lock().unwrap().is_none());
}
