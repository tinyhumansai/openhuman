//! SSE-focused tests: the incremental frame parser, the reconnect dedupe
//! cursor, and end-to-end streaming (including decode/empty/connect edges)
//! driven through the shared TCP stub.

use super::{http_json, spawn_stub};
use crate::openhuman::medulla::client::sse::{SeqDedup, SseFrame, SseParser};
use crate::openhuman::medulla::client::*;
use futures::StreamExt;

// ---------------------------------------------------------------------------
// SSE parser
// ---------------------------------------------------------------------------

fn parse_all(input: &str) -> Vec<SseFrame> {
    let mut parser = SseParser::new();
    let mut out = Vec::new();
    parser.feed(input, &mut out);
    out
}

#[test]
fn parses_id_and_data_frame() {
    let frames = parse_all("id: 42\ndata: {\"a\":1}\n\n");
    assert_eq!(
        frames,
        vec![SseFrame {
            id: Some(42),
            data: "{\"a\":1}".to_string(),
        }]
    );
}

#[test]
fn ignores_ping_comments() {
    let frames = parse_all(": ping\n\ndata: hi\n\n");
    assert_eq!(
        frames,
        vec![SseFrame {
            id: None,
            data: "hi".to_string(),
        }]
    );
}

#[test]
fn concatenates_multiline_data() {
    let frames = parse_all("data: line1\ndata: line2\n\n");
    assert_eq!(
        frames,
        vec![SseFrame {
            id: None,
            data: "line1\nline2".to_string(),
        }]
    );
}

#[test]
fn handles_chunked_and_crlf_boundaries() {
    let mut parser = SseParser::new();
    let mut out = Vec::new();
    // Split a single frame across several feeds, with CRLF line endings.
    parser.feed("id: 7\r\nda", &mut out);
    parser.feed("ta: {\"x\":", &mut out);
    parser.feed("2}\r\n\r\n", &mut out);
    assert_eq!(
        out,
        vec![SseFrame {
            id: Some(7),
            data: "{\"x\":2}".to_string(),
        }]
    );
}

#[test]
fn yields_multiple_frames() {
    let frames = parse_all("id: 1\ndata: a\n\nid: 2\ndata: b\n\n");
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].id, Some(1));
    assert_eq!(frames[1].id, Some(2));
}

// ---------------------------------------------------------------------------
// Reconnect dedupe cursor
// ---------------------------------------------------------------------------

#[test]
fn dedupe_skips_replayed_seqs_and_advances_cursor() {
    let mut d = SeqDedup::new(Some(2));
    assert!(!d.accept(Some(1))); // replayed, below cursor
    assert!(!d.accept(Some(2))); // equal to cursor
    assert!(d.accept(Some(3))); // new
    assert_eq!(d.cursor(), Some(3));
    assert!(d.accept(None)); // deltas always pass
    assert!(!d.accept(Some(3))); // duplicate
    assert!(d.accept(Some(4)));
    assert_eq!(d.cursor(), Some(4));
}

#[test]
fn dedupe_from_start_accepts_everything() {
    let mut d = SeqDedup::new(None);
    assert!(d.accept(Some(0)));
    assert!(d.accept(Some(1)));
    assert!(!d.accept(Some(1)));
}

// ---------------------------------------------------------------------------
// Integration: SSE streaming through the TCP stub
// ---------------------------------------------------------------------------

#[tokio::test]
async fn integration_sse_stream_yields_frames() {
    // SSE response: headers, then two persisted event frames, then close.
    let frame = |seq: u64, body: &str| {
        format!(
            "id: {seq}\ndata: {{\"seq\":{seq},\"at\":1,\"sessionId\":\"s1\",\"event\":{{\"kind\":\"assistant\",\"body\":\"{body}\"}}}}\n\n"
        )
    };
    let mut response =
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n".to_vec();
    response.extend_from_slice(": ping\n\n".as_bytes());
    response.extend_from_slice(frame(1, "one").as_bytes());
    response.extend_from_slice(frame(2, "two").as_bytes());
    let base = spawn_stub(response).await;

    let client = MedullaClient::new(base, "jwt-abc");
    let stream = client.stream_events("s1", None);
    futures::pin_mut!(stream);

    let first = stream.next().await.unwrap().unwrap();
    assert_eq!(first.seq, Some(1));
    assert_eq!(first.kind(), EventKind::Assistant { body: "one".into() });

    let second = stream.next().await.unwrap().unwrap();
    assert_eq!(second.seq, Some(2));
    assert_eq!(second.kind(), EventKind::Assistant { body: "two".into() });
    // Stop by dropping the stream.
}

// ---------------------------------------------------------------------------
// SSE stream edge cases (decode error, empty-data skip, connect failure)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_surfaces_a_decode_error_for_bad_json() {
    let mut response =
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n".to_vec();
    response.extend_from_slice(b": ping\n\n");
    response.extend_from_slice(b"data: {not valid json}\n\n");
    let base = spawn_stub(response).await;
    let client = MedullaClient::new(base, "jwt");
    let stream = client.stream_events("s1", None);
    futures::pin_mut!(stream);
    let first = stream.next().await.unwrap();
    assert!(matches!(first, Err(ClientError::Decode(_))), "{first:?}");
}

#[tokio::test]
async fn sse_skips_empty_data_frames() {
    let mut response =
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n".to_vec();
    // An id-only frame and an empty-data frame both carry no payload → skipped.
    response.extend_from_slice(b"data: \n\n");
    response.extend_from_slice(
        b"id: 5\ndata: {\"seq\":5,\"at\":1,\"sessionId\":\"s1\",\"event\":{\"kind\":\"assistant\",\"body\":\"real\"}}\n\n",
    );
    let base = spawn_stub(response).await;
    let client = MedullaClient::new(base, "jwt");
    let stream = client.stream_events("s1", None);
    futures::pin_mut!(stream);
    let first = stream.next().await.unwrap().unwrap();
    assert_eq!(first.seq, Some(5));
    assert_eq!(
        first.kind(),
        EventKind::Assistant {
            body: "real".into()
        }
    );
}

#[tokio::test]
async fn sse_connect_failure_surfaces_transport_error() {
    // A non-2xx status on the stream GET fails `error_for_status`.
    let base = spawn_stub(http_json("HTTP/1.1 500 Internal Server Error", "boom")).await;
    let client = MedullaClient::new(base, "jwt");
    let stream = client.stream_events("s1", None);
    futures::pin_mut!(stream);
    let first = stream.next().await.unwrap();
    assert!(matches!(first, Err(ClientError::Transport(_))), "{first:?}");
}
