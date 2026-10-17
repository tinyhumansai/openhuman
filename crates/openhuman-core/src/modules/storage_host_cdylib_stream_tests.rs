//! Streamed blobs and watch signals against a REAL loaded module (#7786).

use super::*;

const MIB: u64 = 1024 * 1024;

impl Rig {
    /// One of the probe's own members.
    async fn member(&self, member: &str, args: impl serde::Serialize) -> Value {
        self.probe
            .call(member, args)
            .await
            .expect("the probe answers")
    }

    fn peer_proxy(&self, n: usize, object: &str) -> Proxy {
        self.peers[n]
            .proxy(
                STORAGE_INTERFACE,
                format!("/ai/tinyhumans/tinystorage/{object}"),
                STORAGE_INTERFACE,
            )
            .unwrap()
            .with_timeout(std::time::Duration::from_secs(10))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_blob_over_the_bus_frame_round_trips_through_a_peer_bound_stream() {
    let _turn = TURN.lock().await;
    let rig = rig("tinyjuice").await;
    // 20 MiB: over the 16 MiB frame no inline call could carry.
    let len = 20 * MIB;
    let stored = rig.member("StreamPut", ("juice/ccr/streamed", len)).await;
    assert_eq!(stored["ok"]["len"], len, "{stored}");
    // It is in the module's own scope, byte for byte.
    let handle = rig.backend.for_scope(&rig.scope("tinyjuice")).unwrap();
    let meta = handle
        .blobs()
        .head("juice/ccr/streamed")
        .await
        .unwrap()
        .expect("the streamed blob is stored");
    assert_eq!(meta.len, len);
    let bytes = handle
        .blobs()
        .get_range("juice/ccr/streamed", 0..1024)
        .await
        .unwrap()
        .expect("range");
    assert!(bytes.iter().all(|byte| *byte == 0xAB));
    // And it comes back whole by `GetStream`.
    let read = rig.member("StreamGet", ("juice/ccr/streamed",)).await;
    assert_eq!(read["ok"]["len"], len, "{read}");
    assert_eq!(read["ok"]["meta_len"], len, "{read}");
    assert_eq!(read["ok"]["all_fill"], true, "{read}");
    // A key that is not there is `null`, not an error.
    let missing = rig.member("StreamGet", ("juice/ccr/absent",)).await;
    assert!(
        missing["ok"].is_null() && missing.get("ok").is_some(),
        "{missing}"
    );
    let trail = rig.trail.0.lock().unwrap().clone();
    assert!(trail
        .iter()
        .any(|r| r.op == "PutStream" && r.outcome == "ok" && r.module == "tinyjuice"));
    assert!(trail
        .iter()
        .any(|r| r.op == "GetStream" && r.outcome == "ok" && r.module == "tinyjuice"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_streamed_blob_is_still_bound_by_the_grant_and_the_length() {
    let _turn = TURN.lock().await;
    let rig = rig("tinyjuice").await;
    // Outside the CCR prefix: refused before anything is stored.
    let outside = rig.member("StreamPut", ("juice/other/x", 2 * MIB)).await;
    assert!(outside.get("err").is_some(), "{outside}");
    let handle = rig.backend.for_scope(&rig.scope("tinyjuice")).unwrap();
    assert!(handle
        .blobs()
        .head("juice/other/x")
        .await
        .unwrap()
        .is_none());
    let denied_get = rig.member("StreamGet", ("juice/other/x",)).await;
    assert!(
        denied_get["err"].to_string().contains("target-not-granted"),
        "{denied_get}"
    );
    // The module keeps working after a refusal.
    let after = rig.member("StreamPut", ("juice/ccr/after", 3 * MIB)).await;
    assert_eq!(after["ok"]["len"], 3 * MIB, "{after}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_inline_blob_no_frame_can_carry_fails_fast_instead_of_hanging() {
    let _turn = TURN.lock().await;
    let rig = rig("tinyjuice").await;
    // 5 000 000 bytes as a JSON array of numbers (`255,` each) is a 20 MB
    // frame, over the 16 MiB cap.
    let started = std::time::Instant::now();
    let big: Value = rig
        .member("PutBlob", ("juice/ccr/over-frame", 5_000_000usize))
        .await;
    assert!(
        big.get("err").is_some(),
        "an oversized frame must fail: {big}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(60),
        "took {:?}",
        started.elapsed()
    );
    let handle = rig.backend.for_scope(&rig.scope("tinyjuice")).unwrap();
    assert!(handle
        .blobs()
        .head("juice/ccr/over-frame")
        .await
        .unwrap()
        .is_none());
    let after: Value = rig.member("PutBlob", ("juice/ccr/after", 2048usize)).await;
    assert!(
        after.get("ok").is_some(),
        "the module keeps working: {after}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_watch_signal_reaches_the_module_that_opened_the_watch() {
    let _turn = TURN.lock().await;
    let rig = rig("tinymcp").await;
    let seen = rig.member("WatchPut", ("mcp.servers", "watched")).await;
    assert_eq!(seen["ok"], "invalidated", "{seen}");
    // A collection the grant never declared cannot be watched.
    let refused = rig
        .call(
            "Documents",
            "WatchOpen",
            json!({"collection": "never_declared", "filter": {"op": "all"}, "interval_ms": 50}),
        )
        .await;
    assert!(denied(&refused, "target-not-granted"), "{refused}");
    let trail = rig.trail.0.lock().unwrap().clone();
    assert!(trail
        .iter()
        .any(|r| r.op == "WatchOpen" && r.outcome == "ok" && r.module == "tinymcp"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_module_cannot_receive_the_first_modules_watch_signals() {
    let _turn = TURN.lock().await;
    // The probe stands for tinymcp; peer 0 stands for tinychannels, which also
    // holds documents.
    let rig = rig_with_peers("tinymcp", &["tinychannels"]).await;
    let theirs = declared("tinychannels").unwrap().collections[0]
        .name
        .clone();
    // The second module listens to every watch signal the host sends anyone
    // (any path), and opens a watch of its own as a positive control.
    let mut everything = rig.peers[0]
        .add_match(
            tinybus::MatchRule::new()
                .signals()
                .interface(tinybus::InterfaceName::new(STORAGE_INTERFACE).unwrap()),
        )
        .await
        .unwrap();
    let documents = rig.peer_proxy(0, "Documents");
    let own: Value = documents
        .call(
            "WatchOpen",
            (json!({"collection": theirs, "filter": {"op": "all"}, "interval_ms": 50}),),
        )
        .await
        .unwrap();
    let own_id = own["watch_id"].as_str().unwrap().to_string();
    // The first module opens a watch and changes its document.
    let first = rig.member("WatchPut", ("mcp.servers", "w1")).await;
    assert_eq!(first["ok"], "invalidated", "{first}");
    // The second module's own change reaches the second module.
    let _: Value = documents
        .call("Put", (put(&theirs, "d", &json!({"v": 1})),))
        .await
        .unwrap();
    let mut ids = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline && !ids.contains(&own_id) {
        if let Ok(Ok(message)) =
            tokio::time::timeout(std::time::Duration::from_millis(200), everything.recv()).await
        {
            let id = message.body.get(0).and_then(|b| b["watch_id"].as_str());
            ids.push(id.map_or_else(
                || format!("<non-watch: {:?}>", message.body),
                str::to_string,
            ));
        }
    }
    assert!(
        ids.contains(&own_id),
        "its own signal never arrived: {ids:?}"
    );
    // Nothing else did: the first module's watch id is not among them.
    assert!(
        ids.iter().all(|id| *id == own_id),
        "a signal for another module's watch leaked: {ids:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_module_cannot_read_or_take_the_first_modules_streams() {
    let _turn = TURN.lock().await;
    // The probe and peer 0 both stand for tinyjuice (same grant, same scope),
    // so only the peer binding keeps the second out; peer 1 stands for tinymcp,
    // which has no blobs at all.
    let rig = rig_with_peers("tinyjuice", &["tinyjuice", "tinymcp"]).await;
    let stored = rig
        .member("StreamPut", ("juice/ccr/private", 2 * MIB))
        .await;
    assert_eq!(stored["ok"]["len"], 2 * MIB, "{stored}");

    // A module without blob rights is refused outright.
    let nosy: tinybus::Result<Value> = rig
        .peer_proxy(1, "Blobs")
        .call("GetStream", (json!({"key": "juice/ccr/private"}),))
        .await;
    match nosy {
        Err(tinybus::Error::MethodFailed { name, message }) => {
            assert_eq!(name, "ai.tinyhumans.tinystorage.Error.Denied");
            assert_eq!(message, "object-not-granted");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }

    // Taking a stream the probe opened toward the host: the peer is granted
    // the key, but the host reads an upload only from the peer that opened it.
    let opened = rig.member("OpenStream", (MIB,)).await;
    let reference: tinybus::StreamRef = serde_json::from_value(opened["ok"].clone())
        .unwrap_or_else(|_| panic!("a stream reference: {opened}"));
    let hijack: tinybus::Result<Value> = rig
        .peer_proxy(0, "Blobs")
        .call(
            "PutStream",
            (json!({"key": "juice/ccr/hijacked", "len": MIB}), reference),
        )
        .await;
    assert!(
        hijack.is_err(),
        "a peer took another peer's stream: {hijack:?}"
    );
    let handle = rig.backend.for_scope(&rig.scope("tinyjuice")).unwrap();
    assert!(handle
        .blobs()
        .head("juice/ccr/hijacked")
        .await
        .unwrap()
        .is_none());
    // The refused attempt did not spend the stream: its owner still uses it.
    let used = rig.member("UseStream", ("juice/ccr/owned", MIB)).await;
    assert!(
        used.get("ok").is_some(),
        "the owner lost its stream: {used}"
    );
    assert_eq!(
        handle
            .blobs()
            .head("juice/ccr/owned")
            .await
            .unwrap()
            .expect("stored by the owner")
            .len,
        MIB
    );

    // A download the host opened toward the probe is not readable by a peer.
    let reply = rig.member("RawGetStream", ("juice/ccr/private",)).await;
    let download: tinybus::StreamRef = serde_json::from_value(reply["ok"][1].clone())
        .unwrap_or_else(|_| panic!("a download reference: {reply}"));
    assert!(
        rig.peers[0]
            .accept_stream_from(&download, &rig.host)
            .is_err(),
        "a peer accepted a stream opened toward another peer"
    );
}
