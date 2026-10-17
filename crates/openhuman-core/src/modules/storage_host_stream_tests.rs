//! Streamed blobs and watch signals, decided by the host without a bus.

use super::*;

fn blob_body(bytes: Vec<u8>) -> BlobReader {
    Box::pin(std::io::Cursor::new(bytes))
}

fn head(key: &str, len: u64) -> PutBlobStreamRequest {
    serde_json::from_value(json!({"key": key, "len": len})).unwrap()
}

#[tokio::test]
async fn a_streamed_blob_is_stored_in_the_callers_scope_and_read_back() {
    let rig = rig();
    let bytes = vec![7u8; 3 * 1024 * 1024];
    let stored = rig
        .host
        .put_blob_stream(
            Some(&alpha()),
            head("files/big", bytes.len() as u64),
            blob_body(bytes.clone()),
        )
        .await
        .unwrap();
    assert_eq!(stored["len"], bytes.len());
    let (meta, mut reader) = rig
        .host
        .get_blob_stream(
            Some(&alpha()),
            serde_json::from_value(json!({"key": "files/big"})).unwrap(),
        )
        .await
        .unwrap()
        .expect("the blob is there");
    assert_eq!(meta.meta.len, bytes.len() as u64);
    let mut read = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut read)
        .await
        .unwrap();
    assert_eq!(read, bytes);
    // Another module never sees it, even by the same key.
    let other = rig
        .host
        .get_blob_stream(
            Some(&beta()),
            serde_json::from_value(json!({"key": "files/big"})).unwrap(),
        )
        .await
        .unwrap();
    assert!(other.is_none());
    let trail = rig.trail.0.lock().unwrap().clone();
    assert!(trail
        .iter()
        .any(|r| r.op == "PutStream" && r.outcome == "ok" && r.module == "alpha"));
}

#[tokio::test]
async fn a_streamed_blob_is_decided_like_any_other_call() {
    let rig = rig();
    let put = |sender: Option<BusName>, key: &'static str, len: u64| {
        let host = &rig.host;
        async move {
            host.put_blob_stream(sender.as_ref(), head(key, len), blob_body(vec![1; 4]))
                .await
        }
    };
    assert_eq!(
        put(None, "files/a", 4).await,
        Err(HostError::Denied("unauthenticated-caller"))
    );
    assert_eq!(
        put(Some(stranger()), "files/a", 4).await,
        Err(HostError::Denied("unknown-caller"))
    );
    assert_eq!(
        put(Some(alpha()), "elsewhere/a", 4).await,
        Err(HostError::Denied("target-not-granted"))
    );
    assert_eq!(
        put(Some(alpha()), "files/a", MAX_STREAMED_BLOB_BYTES + 1).await,
        Err(HostError::Denied("blob-too-large"))
    );
    let missing = rig
        .host
        .get_blob_stream(
            Some(&alpha()),
            serde_json::from_value(json!({"key": "elsewhere/a"})).unwrap(),
        )
        .await;
    assert!(matches!(
        missing,
        Err(HostError::Denied("target-not-granted"))
    ));
    // Nothing was stored by any of the refusals.
    let scope = rig.env.module_scope("alpha").unwrap();
    assert!(rig
        .backend
        .for_scope(&scope)
        .unwrap()
        .blobs()
        .head("files/a")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_read_only_grant_cannot_stream_a_blob_in() {
    let mut read_only = grant("notes");
    read_only.writable = false;
    let rig = rig_with("alpha", read_only);
    let refused = rig
        .host
        .put_blob_stream(Some(&alpha()), head("files/a", 4), blob_body(vec![1; 4]))
        .await;
    assert_eq!(refused, Err(HostError::Denied("read-only-grant")));
}

#[tokio::test]
async fn a_watch_signal_reaches_only_the_callers_own_relay() {
    let rig = rig();
    // The bus endpoint asks for each caller's signals before its first
    // `WatchOpen` reaches the host.
    let mut alpha_signals = rig.host.watch_signals(&alpha()).unwrap();
    let mut beta_signals = rig.host.watch_signals(&beta()).unwrap();
    let opened = call(
        &rig,
        &alpha(),
        Object::Documents,
        "WatchOpen",
        json!({"collection": "notes", "filter": {"op": "all"}, "interval_ms": 20}),
    )
    .await
    .unwrap();
    let watch_id = opened["watch_id"].as_str().unwrap().to_string();
    // Bind beta too, so a leak could only come from the host's routing.
    call(&rig, &beta(), Object::Documents, "Collections", json!({}))
        .await
        .unwrap();
    call(
        &rig,
        &alpha(),
        Object::Documents,
        "Put",
        put("notes", "a", json!({"v": 1})),
    )
    .await
    .unwrap();
    let signal = tokio::time::timeout(std::time::Duration::from_secs(5), alpha_signals.recv())
        .await
        .expect("a signal arrives")
        .unwrap();
    assert_eq!(signal.watch_id, watch_id);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(200), beta_signals.recv())
            .await
            .is_err(),
        "another module's relay must stay silent"
    );
    // A collection outside the grant cannot be watched.
    assert_eq!(
        call(
            &rig,
            &alpha(),
            Object::Documents,
            "WatchOpen",
            json!({"collection": "other", "filter": {"op": "all"}, "interval_ms": 20}),
        )
        .await,
        denied("target-not-granted")
    );
}
