use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tinyinference_local::service::{LocalAiService, RuntimeConfig};

struct BlockedEndpoint {
    base_url: String,
    hits: Arc<AtomicUsize>,
    accepted: tokio::sync::oneshot::Receiver<()>,
    release: tokio::sync::oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}

async fn spawn_blocked_counting_endpoint() -> BlockedEndpoint {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let server_hits = Arc::clone(&hits);
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut accepted_tx = Some(accepted_tx);
        let mut release_rx = Some(release_rx);
        while let Ok((stream, _)) = listener.accept().await {
            let hit = server_hits.fetch_add(1, Ordering::SeqCst);
            if hit == 0 {
                if let Some(sender) = accepted_tx.take() {
                    let _ = sender.send(());
                }
                if let Some(receiver) = release_rx.take() {
                    let _ = receiver.await;
                }
            }
            drop(stream);
        }
    });
    BlockedEndpoint {
        base_url: format!("http://127.0.0.1:{}", addr.port()),
        hits,
        accepted: accepted_rx,
        release: release_tx,
        server: task,
    }
}

#[tokio::test]
async fn local_ai_status_poll_burst_coalesces_probe_and_preserves_current_provider() {
    let endpoint = spawn_blocked_counting_endpoint().await;
    let initial_runtime = RuntimeConfig::default();
    let service = Arc::new(LocalAiService::new(&initial_runtime));
    let mut current_runtime = initial_runtime;
    current_runtime.local_ai.runtime_enabled = true;
    current_runtime.local_ai.opt_in_confirmed = true;
    current_runtime.local_ai.provider = "lm_studio".to_string();
    current_runtime.local_ai.base_url = Some(endpoint.base_url.clone());

    let (snapshot, first_task) =
        local_ai_status_snapshot(Arc::clone(&service), current_runtime.clone());
    assert_eq!(snapshot.provider, "lm_studio");
    let first_task = first_task.expect("first poll schedules a probe");
    endpoint.accepted.await.unwrap();

    for _ in 1..8 {
        let (_, task) = local_ai_status_snapshot(Arc::clone(&service), current_runtime.clone());
        assert!(task.is_none(), "polls during a probe coalesce");
    }
    assert_eq!(endpoint.hits.load(Ordering::SeqCst), 1);

    endpoint.release.send(()).unwrap();
    first_task.await.unwrap();
    assert_eq!(endpoint.hits.load(Ordering::SeqCst), 1);

    let (_, retry_task) = local_ai_status_snapshot(service, current_runtime);
    retry_task
        .expect("poll after an unreachable result schedules a fresh probe")
        .await
        .unwrap();
    assert_eq!(endpoint.hits.load(Ordering::SeqCst), 2);
    endpoint.server.abort();
}
