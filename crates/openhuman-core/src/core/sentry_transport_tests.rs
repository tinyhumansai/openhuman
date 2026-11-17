use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;

fn event_envelope(message: &str) -> Envelope {
    let mut envelope = Envelope::new();
    envelope.add_item(sentry::protocol::Event {
        message: Some(message.to_owned()),
        ..Default::default()
    });
    envelope
}

#[test]
fn sends_authenticated_envelope_and_applies_category_rate_limit() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let request = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let read = stream.read(&mut buffer).unwrap();
            bytes.extend_from_slice(&buffer[..read]);
            if read < buffer.len() {
                break;
            }
        }
        stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nX-Sentry-Rate-Limits: 60:error:project\r\nContent-Length: 0\r\n\r\n",
                )
                .unwrap();
        String::from_utf8(bytes).unwrap()
    });
    let options = ClientOptions {
        dsn: Some(format!("http://public@{address}/1").parse().unwrap()),
        ..Default::default()
    };
    let transport = SharedReqwestTransport::new(&options);
    transport.send_envelope(event_envelope("first"));
    assert!(transport.flush(Duration::from_secs(3)));
    transport.send_envelope(event_envelope("rate-limited"));
    assert!(transport.flush(Duration::from_secs(3)));

    let request = request.join().unwrap();
    assert!(request.contains("POST /api/1/envelope/ HTTP/1.1"));
    assert!(request.contains("x-sentry-auth: Sentry"));
    assert!(request.contains("\"message\":\"first\""));
    assert!(!request.contains("rate-limited"));
}

#[test]
fn send_failure_does_not_prevent_flush() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let options = ClientOptions {
        dsn: Some(format!("http://public@{address}/1").parse().unwrap()),
        ..Default::default()
    };
    let transport = SharedReqwestTransport::new(&options);
    transport.send_envelope(event_envelope("unreachable"));
    assert!(transport.flush(Duration::from_secs(3)));
    assert!(transport.shutdown(Duration::from_secs(3)));
}

#[test]
fn an_invalid_proxy_is_ignored_rather_than_failing_the_transport() {
    let options = ClientOptions {
        dsn: Some("http://public@127.0.0.1:9/1".parse().unwrap()),
        http_proxy: Some("http://[not a proxy".into()),
        https_proxy: Some("http://[not a proxy".into()),
        debug: true,
        ..Default::default()
    };
    let transport = SharedReqwestTransport::new(&options);
    transport.shutdown(Duration::from_secs(1));
}
