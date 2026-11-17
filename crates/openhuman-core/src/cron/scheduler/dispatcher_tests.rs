use super::*;

#[test]
fn manual_worker_owner_survives_the_invocations_runtime() {
    let caller = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (release, wait) = tokio::sync::oneshot::channel();
    let (finished, result) = std::sync::mpsc::channel();
    caller.block_on(async {
        spawn_manual_owner(async move {
            let _ = wait.await;
            let _ = finished.send(());
        })
        .unwrap();
    });
    drop(caller);
    let _ = release.send(());
    result
        .recv_timeout(Duration::from_secs(5))
        .expect("manual completion and acknowledgement outlive the invocation runtime");
}
