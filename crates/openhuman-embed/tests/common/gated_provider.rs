//! An OpenAI-compatible provider that can hold its replies until enough chat
//! requests are in flight at once.
//!
//! `wiremock` cannot do this: its responders are synchronous and its server
//! runs on one current-thread runtime, so a responder that waited would stall
//! every other request. This is a minimal HTTP/1.1 server on the test's own
//! runtime instead, just enough for the provider client's
//! `POST /v1/chat/completions` (JSON in, JSON out, keep-alive).
//!
//! The gate turns "the turns overlap" from a wall-clock claim into a
//! structural one. Armed with [`GatedProvider::hold_until`], every chat
//! request waits at the gate until `n` of them are waiting together, then the
//! gate opens for good. A host that runs turns one after another can never
//! have two requests in flight, so it never opens the gate, whatever the
//! machine's speed; a host that overlaps them opens it as soon as the `n`-th
//! request arrives.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

#[derive(Default)]
struct Counters {
    /// Chat requests received and not yet answered.
    in_flight: usize,
    /// The most chat requests ever in flight at once.
    peak_in_flight: usize,
    /// Every chat request received.
    chat_requests: usize,
    /// While `Some(n)`, chat requests wait until `n` are in flight.
    hold_until: Option<usize>,
}

struct Shared {
    counters: Mutex<Counters>,
    gate_opened: Notify,
    reply: String,
}

impl Shared {
    fn counters(&self) -> std::sync::MutexGuard<'_, Counters> {
        self.counters.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A running gated provider. The server task lives as long as the runtime.
pub struct GatedProvider {
    addr: SocketAddr,
    shared: Arc<Shared>,
}

impl GatedProvider {
    /// Starts a provider answering every chat completion with `reply`. The
    /// gate starts open: requests are answered as they arrive.
    pub async fn start(reply: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the gated provider");
        let addr = listener.local_addr().expect("gated provider address");
        let shared = Arc::new(Shared {
            counters: Mutex::new(Counters::default()),
            gate_opened: Notify::new(),
            reply: super::chat_completion(reply).to_string(),
        });
        let server = Arc::clone(&shared);
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(serve_connection(stream, Arc::clone(&server)));
            }
        });
        Self { addr, shared }
    }

    /// `http://127.0.0.1:<port>`, like `MockServer::uri`.
    pub fn uri(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Closes the gate: from now on every chat request waits until `n` chat
    /// requests are in flight together, which opens it for good.
    pub fn hold_until(&self, n: usize) {
        assert!(n > 0, "a gate that waits for no request never closes");
        self.shared.counters().hold_until = Some(n);
    }

    /// Whether the gate armed by [`Self::hold_until`] has opened (or was
    /// never armed).
    pub fn is_open(&self) -> bool {
        self.shared.counters().hold_until.is_none()
    }

    /// The most chat requests that were ever in flight at once.
    pub fn peak_in_flight(&self) -> usize {
        self.shared.counters().peak_in_flight
    }

    /// Chat requests received and not yet answered.
    pub fn in_flight(&self) -> usize {
        self.shared.counters().in_flight
    }

    /// Every chat request received so far.
    pub fn chat_requests(&self) -> usize {
        self.shared.counters().chat_requests
    }
}

/// One keep-alive connection: answer requests until the client hangs up.
async fn serve_connection(stream: TcpStream, shared: Arc<Shared>) {
    let mut stream = BufReader::new(stream);
    loop {
        let Some((method, path)) = read_request(&mut stream).await else {
            return;
        };
        let is_chat = method == "POST" && path.ends_with("/chat/completions");
        let response = if is_chat {
            pass_gate(&shared).await;
            http_response("200 OK", &shared.reply)
        } else {
            // What wiremock answers for a route nothing was mounted on (the
            // provider client's `/models` discovery, for one).
            http_response("404 Not Found", "")
        };
        let written = stream.get_mut().write_all(response.as_bytes()).await;
        if is_chat {
            shared.counters().in_flight -= 1;
        }
        if written.is_err() {
            return;
        }
    }
}

/// Counts a chat request in, and waits while the gate is closed.
async fn pass_gate(shared: &Shared) {
    {
        let mut counters = shared.counters();
        counters.chat_requests += 1;
        counters.in_flight += 1;
        counters.peak_in_flight = counters.peak_in_flight.max(counters.in_flight);
        if counters
            .hold_until
            .is_some_and(|needed| counters.in_flight >= needed)
        {
            log::debug!(
                "[gated_provider] gate opened with {} chat requests in flight",
                counters.in_flight
            );
            counters.hold_until = None;
            shared.gate_opened.notify_waiters();
        }
    }
    loop {
        let opened = shared.gate_opened.notified();
        tokio::pin!(opened);
        // Register before checking, so an opening between the check and the
        // await is not missed.
        opened.as_mut().enable();
        if shared.counters().hold_until.is_none() {
            return;
        }
        opened.await;
    }
}

/// Reads one request's head and body. `None` when the client closed the
/// connection (or sent something this server does not speak).
async fn read_request(stream: &mut BufReader<TcpStream>) -> Option<(String, String)> {
    let mut request_line = String::new();
    if stream.read_line(&mut request_line).await.ok()? == 0 {
        return None;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();

    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        if stream.read_line(&mut header).await.ok()? == 0 {
            return None;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':')?;
        let name = name.trim();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.trim().parse().ok()?;
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            // The provider client sends JSON with a length; a chunked body
            // would mean this server is no longer a faithful stand-in.
            panic!("gated provider: unsupported transfer-encoding {value:?} on {method} {path}");
        }
    }
    let mut body = vec![0u8; content_length];
    stream.read_exact(&mut body).await.ok()?;
    Some((method, path))
}

fn http_response(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
        body.len()
    )
}
