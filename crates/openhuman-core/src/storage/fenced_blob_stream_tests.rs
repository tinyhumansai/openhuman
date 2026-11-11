//! Streaming behavior of the lease-guarded v2 blob port.

#![cfg(feature = "storage-sqlite")]

use super::*;
use crate::storage::fence::{fence_error, FenceError, FenceRegistry, LeaseFence, ScopeMatch};
use crate::storage::lease::{DocumentLeases, LeaseStore};
use crate::storage::test_support::sqlite_backend;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tinystoragedrivers_types::Scope;

async fn guarded_blobs(
    now: Arc<AtomicU64>,
) -> (crate::storage::test_support::SqliteBackend, GuardedBlobs) {
    let fixture = sqlite_backend().unwrap();
    let leases = Arc::new(
        DocumentLeases::cluster(
            fixture.backend.as_ref(),
            "node-a",
            None,
            Duration::from_secs(30),
        )
        .unwrap(),
    );
    let grant = leases.acquire("alice", 0).await.unwrap();
    let fence = Arc::new(LeaseFence::new(
        Arc::clone(&leases) as Arc<dyn LeaseStore>,
        "alice",
        "node-a",
        grant.epoch,
        grant.expires_at_ms,
        0,
        vec![ScopeMatch::Exact("profile:alice".into())],
    ));
    let clock = Arc::new(FenceRegistry::new({
        let now = Arc::clone(&now);
        move || now.load(Ordering::SeqCst)
    }));
    let scoped = fixture
        .backend
        .for_scope(&Scope::new("profile:alice").unwrap())
        .unwrap();
    let blobs = GuardedBlobs {
        inner: Arc::clone(scoped.blobs()),
        fence,
        clock,
    };
    (fixture, blobs)
}

#[tokio::test]
async fn a_live_fence_forwards_streamed_blob_writes() {
    let now = Arc::new(AtomicU64::new(1));
    let (_fixture, blobs) = guarded_blobs(Arc::clone(&now)).await;
    let reader = Box::pin(futures::stream::iter([Ok(bytes::Bytes::from_static(
        b"payload",
    ))]));

    let meta = blobs
        .put_stream("live", reader, 7, Some("text/plain"))
        .await
        .unwrap();
    assert_eq!(meta.len, 7);
    assert_eq!(blobs.get("live").await.unwrap().unwrap().bytes, b"payload");
}

#[tokio::test]
async fn an_expired_fence_rejects_a_stream_before_polling_it() {
    let now = Arc::new(AtomicU64::new(1));
    let (_fixture, blobs) = guarded_blobs(Arc::clone(&now)).await;
    // The lease expires at 30 seconds. At the exact deadline the host check
    // rejects before the stream reaches its first item.
    now.store(30_000, Ordering::SeqCst);
    let polls = Arc::new(AtomicU64::new(0));
    let observed = Arc::clone(&polls);
    let reader = Box::pin(futures::stream::poll_fn(move |_| {
        observed.fetch_add(1, Ordering::SeqCst);
        std::task::Poll::Ready(Some(Ok(bytes::Bytes::from_static(b"x"))))
    }));

    let error = blobs
        .put_stream("stale", reader, 1, None)
        .await
        .unwrap_err();
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        fence_error(&error),
        Some(FenceError::Expired { .. })
    ));
}
