//! A scriptable, recording fake backend for SaaS end-to-end tests.
//!
//! Every request is recorded with its path, `Authorization` header and body,
//! so a test can check which credential carried which content. Inference
//! (`…/chat/completions`) is answered from the last message:
//!
//! - after a tool result: `TOOL-RESULT: <the tool's output>`;
//! - a user message containing [`HANG`]: the request is held open until the
//!   peer goes away, so the turn stays in flight until it is cancelled;
//! - a user message containing `PROBE-TOOL <name> <json args>`: one call to
//!   that tool;
//! - anything else: `ECHO: <the user message>`.
//!
//! `HANG` and `PROBE-TOOL` apply only to agent turns (requests that offer
//! tools); a side completion such as title generation always gets the echo.
//! Both streaming (SSE) and plain JSON completions are served. Anything that
//! is not inference gets an immediate `500`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

/// Marks a user message whose turn must stay in flight.
pub const HANG: &str = "HANG-TURN";
/// Marks a user message whose turn calls a tool: `PROBE-TOOL <name> <json>`.
pub const PROBE: &str = "PROBE-TOOL";

/// One request the backend received.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub path: String,
    pub auth: String,
    pub body: String,
}

impl Recorded {
    pub fn is_inference(&self) -> bool {
        self.path.contains("/chat/completions")
    }

    /// Tool names this inference request offered the model.
    pub fn offered_tools(&self) -> Vec<String> {
        let body: Value = serde_json::from_str(&self.body).unwrap_or(Value::Null);
        body["tools"]
            .as_array()
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|t| {
                        t.pointer("/function/name")
                            .or_else(|| t.get("name"))
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The running fake backend.
#[derive(Clone)]
pub struct MockLlm {
    pub port: u16,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
}

impl MockLlm {
    pub fn recorded(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
}

pub fn mock_llm() -> MockLlm {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let sink = requests.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let sink = sink.clone();
            std::thread::spawn(move || serve(stream, &sink));
        }
    });
    MockLlm { port, requests }
}

enum Reply {
    Hang,
    Text(String),
    Tool { name: String, arguments: String },
}

fn serve(stream: TcpStream, sink: &Mutex<Vec<Recorded>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
    let (mut auth, mut length, mut chunked) = (String::new(), 0usize, false);
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            let value = value.trim();
            if name.eq_ignore_ascii_case("authorization") {
                auth = value.to_string();
            } else if name.eq_ignore_ascii_case("content-length") {
                length = value.parse().unwrap_or(0);
            } else if name.eq_ignore_ascii_case("transfer-encoding") {
                chunked = value.eq_ignore_ascii_case("chunked");
            }
        }
    }
    let body = if chunked {
        read_chunked(&mut reader)
    } else {
        let mut buf = vec![0u8; length];
        let _ = reader.read_exact(&mut buf);
        buf
    };
    let body = String::from_utf8_lossy(&body).into_owned();
    let recorded = Recorded { path, auth, body };
    sink.lock().unwrap().push(recorded.clone());

    let mut stream = stream;
    if !recorded.is_inference() {
        let _ = stream.write_all(
            b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
        );
        return;
    }
    let request: Value = serde_json::from_str(&recorded.body).unwrap_or(Value::Null);
    let reply = reply_for(&request);
    if matches!(reply, Reply::Hang) {
        // Hold the request until the peer goes away (a cancel drops it).
        let mut buf = [0u8; 4096];
        while matches!(reader.read(&mut buf), Ok(n) if n > 0) {}
        return;
    }
    let streaming = request["stream"].as_bool().unwrap_or(false);
    let model = request["model"].as_str().unwrap_or("mock").to_string();
    let response = if streaming {
        sse_response(&reply, &model)
    } else {
        json_response(&reply, &model)
    };
    let _ = stream.write_all(response.as_bytes());
}

fn read_chunked(reader: &mut BufReader<TcpStream>) -> Vec<u8> {
    let mut body = Vec::new();
    loop {
        let mut size = String::new();
        if reader.read_line(&mut size).unwrap_or(0) == 0 {
            break;
        }
        let size =
            usize::from_str_radix(size.trim().split(';').next().unwrap_or("0"), 16).unwrap_or(0);
        if size == 0 {
            let mut trailer = String::new();
            let _ = reader.read_line(&mut trailer);
            break;
        }
        let mut chunk = vec![0u8; size + 2];
        if reader.read_exact(&mut chunk).is_err() {
            break;
        }
        body.extend_from_slice(&chunk[..size]);
    }
    body
}

/// The text of a message's `content`, string or parts.
pub fn content_text(message: &Value) -> String {
    match &message["content"] {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn reply_for(request: &Value) -> Reply {
    let messages = request["messages"].as_array().cloned().unwrap_or_default();
    let agent_turn = request["tools"].as_array().is_some_and(|t| !t.is_empty());
    if let Some(last) = messages.last() {
        if last["role"] == "tool" {
            let output: String = content_text(last).chars().take(4000).collect();
            return Reply::Text(format!("TOOL-RESULT: {output}"));
        }
    }
    let text = messages
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .map(content_text)
        .unwrap_or_default();
    if text.contains(HANG) {
        return Reply::Hang;
    }
    if agent_turn {
        if let Some(rest) = text.split(PROBE).nth(1) {
            let rest = rest.trim_start();
            let line = rest.lines().next().unwrap_or("");
            let (name, arguments) = line.split_once(' ').unwrap_or((line, "{}"));
            return Reply::Tool {
                name: name.to_string(),
                arguments: arguments.trim().to_string(),
            };
        }
    }
    let echoed: String = text.chars().take(2000).collect();
    Reply::Text(format!("ECHO: {echoed}"))
}

fn usage() -> Value {
    json!({ "prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18 })
}

fn json_response(reply: &Reply, model: &str) -> String {
    let message = match reply {
        Reply::Text(text) => json!({ "role": "assistant", "content": text }),
        Reply::Tool { name, arguments } => json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": "call_probe",
                "type": "function",
                "function": { "name": name, "arguments": arguments }
            }]
        }),
        Reply::Hang => unreachable!(),
    };
    let finish = if matches!(reply, Reply::Tool { .. }) {
        "tool_calls"
    } else {
        "stop"
    };
    let body = json!({
        "id": "chatcmpl-mock",
        "object": "chat.completion",
        "created": 0,
        "model": model,
        "choices": [{ "index": 0, "message": message, "finish_reason": finish }],
        "usage": usage(),
    })
    .to_string();
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn sse_response(reply: &Reply, model: &str) -> String {
    let chunk = |delta: Value, finish: Option<&str>, usage: Option<Value>| {
        let mut envelope = json!({
            "id": "chatcmpl-mock",
            "object": "chat.completion.chunk",
            "created": 0,
            "model": model,
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
        });
        if let Some(usage) = usage {
            envelope["usage"] = usage;
        }
        format!("data: {envelope}\n\n")
    };
    let mut out = String::from(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n",
    );
    let finish = match reply {
        Reply::Text(text) => {
            out.push_str(&chunk(
                json!({ "role": "assistant", "content": text }),
                None,
                None,
            ));
            "stop"
        }
        Reply::Tool { name, arguments } => {
            out.push_str(&chunk(
                json!({ "role": "assistant", "tool_calls": [{
                    "index": 0,
                    "id": "call_probe",
                    "type": "function",
                    "function": { "name": name, "arguments": arguments }
                }]}),
                None,
                None,
            ));
            "tool_calls"
        }
        Reply::Hang => unreachable!(),
    };
    out.push_str(&chunk(json!({}), Some(finish), Some(usage())));
    out.push_str("data: [DONE]\n\n");
    out
}
