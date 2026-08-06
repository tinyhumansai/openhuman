//! Sentry envelope transport backed by the core's shared reqwest 0.12 stack.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use sentry::transports::{RateLimiter, RateLimitingCategory};
use sentry::{sentry_debug, ClientOptions, Envelope, Transport};

enum Task {
    Send(Envelope),
    Flush(mpsc::SyncSender<()>),
    Shutdown,
}

/// A bounded, nonblocking Sentry transport using reqwest 0.12.
pub struct SharedReqwestTransport {
    sender: mpsc::SyncSender<Task>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl SharedReqwestTransport {
    /// Build a transport from the options consumed by Sentry's HTTP transport.
    pub fn new(options: &ClientOptions) -> Self {
        let mut builder = reqwest::blocking::Client::builder();
        if options.accept_invalid_certs {
            builder = builder.danger_accept_invalid_certs(true);
        }
        if let Some(url) = options.http_proxy.as_ref() {
            match reqwest::Proxy::http(url.as_ref()) {
                Ok(proxy) => builder = builder.proxy(proxy),
                Err(error) => sentry_debug!("invalid HTTP proxy: {error:?}"),
            }
        }
        if let Some(url) = options.https_proxy.as_ref() {
            match reqwest::Proxy::https(url.as_ref()) {
                Ok(proxy) => builder = builder.proxy(proxy),
                Err(error) => sentry_debug!("invalid HTTPS proxy: {error:?}"),
            }
        }
        let client = builder
            .build()
            .expect("reqwest 0.12 TLS client must be available");
        let dsn = options
            .dsn
            .as_ref()
            .expect("Sentry transport requires a DSN");
        let auth = dsn.to_auth(Some(&options.user_agent)).to_string();
        let url = dsn.envelope_api_url().to_string();
        let (sender, receiver) = mpsc::sync_channel(30);
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = shutdown.clone();

        let handle = std::thread::Builder::new()
            .name("sentry-transport".into())
            .spawn(move || {
                let mut rate_limiter = RateLimiter::new();
                for task in receiver {
                    if worker_shutdown.load(Ordering::SeqCst) {
                        return;
                    }
                    let envelope = match task {
                        Task::Send(envelope) => envelope,
                        Task::Flush(done) => {
                            let _ = done.send(());
                            continue;
                        }
                        Task::Shutdown => return,
                    };
                    if rate_limiter
                        .is_disabled(RateLimitingCategory::Any)
                        .is_some()
                    {
                        sentry_debug!("Sentry envelope discarded due to global rate limit");
                        continue;
                    }
                    let Some(envelope) = rate_limiter.filter_envelope(envelope) else {
                        sentry_debug!("Sentry envelope discarded due to category rate limit");
                        continue;
                    };
                    let mut body = Vec::new();
                    if let Err(error) = envelope.to_writer(&mut body) {
                        sentry_debug!("failed to serialize Sentry envelope: {error}");
                        continue;
                    }
                    match client
                        .post(&url)
                        .header("X-Sentry-Auth", &auth)
                        .body(body)
                        .send()
                    {
                        Ok(response) => {
                            if let Some(value) = response
                                .headers()
                                .get("x-sentry-rate-limits")
                                .and_then(|value| value.to_str().ok())
                            {
                                rate_limiter.update_from_sentry_header(value);
                            } else if let Some(value) = response
                                .headers()
                                .get(reqwest::header::RETRY_AFTER)
                                .and_then(|value| value.to_str().ok())
                            {
                                rate_limiter.update_from_retry_after(value);
                            } else if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                                rate_limiter.update_from_429();
                            }
                            if response.status() == reqwest::StatusCode::PAYLOAD_TOO_LARGE {
                                sentry_debug!("Sentry envelope rejected as too large");
                            }
                        }
                        Err(error) => sentry_debug!("failed to send Sentry envelope: {error}"),
                    }
                }
            })
            .ok();
        Self {
            sender,
            shutdown,
            handle,
        }
    }
}

impl Transport for SharedReqwestTransport {
    fn send_envelope(&self, envelope: Envelope) {
        if let Err(error) = self.sender.try_send(Task::Send(envelope)) {
            sentry_debug!("Sentry envelope dropped: {error}");
        }
    }

    fn flush(&self, timeout: Duration) -> bool {
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        self.sender.send(Task::Flush(done_tx)).is_ok() && done_rx.recv_timeout(timeout).is_ok()
    }

    fn shutdown(&self, timeout: Duration) -> bool {
        let flushed = self.flush(timeout);
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = self.sender.send(Task::Shutdown);
        flushed
    }
}

impl Drop for SharedReqwestTransport {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = self.sender.send(Task::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Factory suitable for [`ClientOptions::transport`].
pub fn factory(options: &ClientOptions) -> Arc<dyn Transport> {
    Arc::new(SharedReqwestTransport::new(options))
}

#[cfg(test)]
mod tests {
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
}
