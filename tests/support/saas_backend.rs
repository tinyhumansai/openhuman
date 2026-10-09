//! The backend side of the SaaS isolation suite: the node mock
//! (`scripts/mock-api-server.mjs`) and a recording pass-through in front of it.
//!
//! Include with `#[path = "support/saas_backend.rs"] mod saas_backend;` next to
//! `support/saas.rs`.

#![allow(dead_code)]

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::saas::free_port;

/// The node mock backend, killed when dropped.
pub struct Mock {
    child: std::process::Child,
    pub origin: String,
}

impl Drop for Mock {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Mock {
    pub fn start(client: &reqwest::blocking::Client) -> Self {
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/mock-api-server.mjs")
            .canonicalize()
            .expect("scripts/mock-api-server.mjs exists");
        let port = free_port();
        let child = Command::new("node")
            .arg(&script)
            .args(["--port", &port.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn `node scripts/mock-api-server.mjs` (is node on PATH?)");
        let mock = Self {
            child,
            origin: format!("http://127.0.0.1:{port}"),
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        while !client
            .get(format!("{}/__admin/health", mock.origin))
            .send()
            .is_ok_and(|r| r.status().is_success())
        {
            assert!(Instant::now() < deadline, "the mock backend never started");
            std::thread::sleep(Duration::from_millis(100));
        }
        mock
    }
}

/// One request the proxy saw.
#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub authorization: String,
    pub body: String,
}

impl Seen {
    pub fn is_inference(&self) -> bool {
        self.method == "POST" && self.path.contains("/chat/completions")
    }
}

/// A recording pass-through in front of the mock. The mock redacts
/// `Authorization` in its own log; the credential each request carried is the
/// point of this suite, so it is captured here before forwarding.
pub struct Proxy {
    pub origin: String,
    seen: std::sync::Arc<std::sync::Mutex<Vec<Seen>>>,
}

impl Proxy {
    pub fn start(upstream: String) -> Self {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = std::sync::Arc::clone(&seen);
        std::thread::spawn(move || {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(120))
                .no_gzip()
                .build()
                .unwrap();
            for stream in listener.incoming().flatten() {
                let (client, upstream, log) = (client.clone(), upstream.clone(), log.clone());
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let mut parts = line.split_whitespace();
                    let method = parts.next().unwrap_or("GET").to_string();
                    let path = parts.next().unwrap_or("/").to_string();
                    let mut headers = Vec::new();
                    let (mut length, mut chunked) = (0usize, false);
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                            break;
                        }
                        if let Some((name, value)) = header.split_once(':') {
                            let (name, value) = (name.trim().to_string(), value.trim().to_string());
                            match name.to_ascii_lowercase().as_str() {
                                "content-length" => length = value.parse().unwrap_or(0),
                                "transfer-encoding" => chunked = value.contains("chunked"),
                                _ => {}
                            }
                            headers.push((name, value));
                        }
                    }
                    let mut body = Vec::new();
                    if chunked {
                        loop {
                            let mut size = String::new();
                            if reader.read_line(&mut size).unwrap_or(0) == 0 {
                                break;
                            }
                            let size = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
                            let mut chunk = vec![0u8; size + 2];
                            if reader.read_exact(&mut chunk).is_err() || size == 0 {
                                break;
                            }
                            body.extend_from_slice(&chunk[..size]);
                        }
                    } else {
                        body.resize(length, 0);
                        let _ = reader.read_exact(&mut body);
                    }
                    let authorization = headers
                        .iter()
                        .find(|(n, _)| n.eq_ignore_ascii_case("authorization"))
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default();
                    log.lock().unwrap().push(Seen {
                        method: method.clone(),
                        path: path.clone(),
                        authorization,
                        body: String::from_utf8_lossy(&body).into_owned(),
                    });

                    let mut request = client.request(
                        reqwest::Method::from_bytes(method.as_bytes()).unwrap(),
                        format!("{upstream}{path}"),
                    );
                    for (name, value) in &headers {
                        if ![
                            "host",
                            "content-length",
                            "connection",
                            "transfer-encoding",
                            "accept-encoding",
                        ]
                        .contains(&name.to_ascii_lowercase().as_str())
                        {
                            request = request.header(name, value);
                        }
                    }
                    let mut stream = stream;
                    match request.body(body).send() {
                        Ok(response) => {
                            let status = response.status().as_u16();
                            let content_type = response
                                .headers()
                                .get("content-type")
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("application/octet-stream")
                                .to_string();
                            let bytes = response.bytes().unwrap_or_default();
                            let _ = write!(
                                stream,
                                "HTTP/1.1 {status} X\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                                bytes.len()
                            );
                            let _ = stream.write_all(&bytes);
                        }
                        Err(_) => {
                            let _ = stream.write_all(
                                b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                            );
                        }
                    }
                });
            }
        });
        Self { origin, seen }
    }

    pub fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}
