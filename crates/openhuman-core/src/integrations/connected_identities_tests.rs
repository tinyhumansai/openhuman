use super::*;

#[test]
fn prompt_identity_reads_do_not_exhaust_the_callers_blocking_pool() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let (done, result) = std::sync::mpsc::channel();
    runtime.spawn(async move {
        let rendered = render_blocking(async {
            tokio::task::spawn_blocking(|| "connected identities".to_string())
                .await
                .unwrap()
        });
        let _ = done.send(rendered);
    });
    let rendered = result.recv_timeout(std::time::Duration::from_secs(5));
    // A failed assertion must not wait forever for the blocked runtime.
    runtime.shutdown_timeout(std::time::Duration::ZERO);
    assert_eq!(rendered.unwrap(), "connected identities");
}
