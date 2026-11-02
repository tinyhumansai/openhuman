//! A stateful mock of the TinyHumans backend's hosted memory routes
//! (`/memory/*`), the wire a SaaS profile's `tinyhumans` memory engine
//! speaks, plus a log of every request it served.
//!
//! It follows `scripts/mock-api/routes/memory.mjs` (the shared node mock)
//! closely enough for learn, list, fetch, recall, explore and forget, with
//! one deliberate difference: **every bearer shares one store**. The real
//! backend keeps each credential's memory apart, which would hide a core
//! that read or wrote outside a profile's `user:<id>` root. Here the only
//! thing keeping profiles apart is the core's upstream `Policy::Clamp`, the
//! worst case `memory::ops` defends (users sharing one engine). Every request
//! is recorded with its bearer and the scopes it names, so a test can check
//! both what each profile read back and what it asked the backend for.
//!
//! Anything outside `/memory/*` answers `500` at once, so a profile's other
//! backend calls fail fast.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{json, Value};

/// One request the mock served.
#[derive(Debug, Clone)]
pub struct MemoryRequest {
    /// `GET`, `POST`, …
    pub method: String,
    /// The path, without the query.
    pub path: String,
    /// A non-reversible identifier of the bearer token
    /// (`Authorization: Bearer <token>`), or empty; see [`credential_id`].
    /// The token itself is never kept.
    pub bearer: String,
    /// Every scope the request named: its body's `scope`, its query's
    /// `scope` or `prefix`.
    pub scopes: Vec<String>,
}

/// The identifier a request log keeps in place of a bearer token: a one-way
/// hash, so a test can tell credentials apart without holding any of them.
pub fn credential_id(token: &str) -> String {
    use std::hash::{Hash, Hasher};
    if token.is_empty() {
        return String::new();
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    token.hash(&mut hasher);
    format!("cred-{:016x}", hasher.finish())
}

#[derive(Debug, Clone)]
struct Event {
    id: String,
    scope: String,
    modality: String,
    wal_offset: u64,
    content: Value,
    context: Value,
}

#[derive(Default)]
struct Store {
    events: Vec<Event>,
    /// Body `idempotency_key` → (fingerprint of what it wrote, event id).
    idempotency: HashMap<String, (String, String)>,
    /// `Idempotency-Key` header claims → the body key that made them.
    claims: HashMap<String, String>,
    packs: HashMap<String, Vec<Value>>,
    next_offset: u64,
    next_id: u64,
    next_pack: u64,
    log: Vec<MemoryRequest>,
}

/// The running mock. Its thread lives as long as the test process.
#[derive(Clone)]
pub struct MockMemory {
    /// The port it listens on (`127.0.0.1`).
    pub port: u16,
    store: Arc<Mutex<Store>>,
}

impl MockMemory {
    /// Starts the mock on a free loopback port.
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the memory mock");
        let port = listener.local_addr().unwrap().port();
        let store = Arc::new(Mutex::new(Store::default()));
        let shared = Arc::clone(&store);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let store = Arc::clone(&shared);
                std::thread::spawn(move || serve(stream, &store));
            }
        });
        Self { port, store }
    }

    /// Every `/memory/*` request served so far.
    pub fn requests(&self) -> Vec<MemoryRequest> {
        self.lock().log.clone()
    }

    /// The scope of every stored event whose text holds `needle`.
    pub fn scopes_holding(&self, needle: &str) -> Vec<String> {
        self.lock()
            .events
            .iter()
            .filter(|event| text_of(&event.content).contains(needle))
            .map(|event| event.scope.clone())
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct Request {
    method: String,
    path: String,
    query: HashMap<String, String>,
    bearer: String,
    claim: Option<String>,
    body: Value,
}

fn read_request(stream: &TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let mut length = 0usize;
    let mut bearer = String::new();
    let mut claim = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).ok()? == 0 || header == "\r\n" {
            break;
        }
        let Some((name, value)) = header.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = value.parse().unwrap_or(0),
            "authorization" => {
                bearer = value
                    .strip_prefix("Bearer ")
                    .unwrap_or("")
                    .trim()
                    .to_string();
            }
            "idempotency-key" => claim = Some(value.to_string()),
            _ => {}
        }
    }
    let mut raw = vec![0u8; length];
    reader.read_exact(&mut raw).ok()?;
    let body = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_string(), parse_query(query)),
        None => (target, HashMap::new()),
    };
    Some(Request {
        method,
        path,
        query,
        bearer,
        claim,
        body,
    })
}

fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            Some((decode(key)?, decode(value)?))
        })
        .collect()
}

/// Percent-decoding (and `+` as a space), enough for scope paths.
fn decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn respond(mut stream: TcpStream, status: u16, body: &Value) {
    let text = body.to_string();
    let reason = if status < 400 { "OK" } else { "Error" };
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
        text.len()
    );
}

fn ok(data: Value) -> (u16, Value) {
    (200, json!({ "success": true, "data": data }))
}

fn fail(status: u16, code: &str) -> (u16, Value) {
    (
        status,
        json!({ "success": false, "error": format!("failed: {code}"), "errorCode": code }),
    )
}

fn serve(stream: TcpStream, store: &Mutex<Store>) {
    let Some(request) = read_request(&stream) else {
        return;
    };
    if !request.path.starts_with("/memory/") {
        respond(stream, 500, &Value::Null);
        return;
    }
    let mut store = store.lock().unwrap_or_else(PoisonError::into_inner);
    let mut scopes = Vec::new();
    if let Some(scope) = request.body.get("scope").and_then(Value::as_str) {
        scopes.push(scope.to_string());
    }
    for key in ["scope", "prefix"] {
        if let Some(scope) = request.query.get(key) {
            scopes.push(scope.clone());
        }
    }
    store.log.push(MemoryRequest {
        method: request.method.clone(),
        path: request.path.clone(),
        bearer: credential_id(&request.bearer),
        scopes,
    });
    let (status, body) = if request.bearer.is_empty() {
        fail(401, "UNAUTHORIZED")
    } else {
        route(&mut store, &request)
    };
    drop(store);
    respond(stream, status, &body);
}

fn text_of(content: &Value) -> String {
    content
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn labels_of(event: &Event) -> Vec<String> {
    event
        .context
        .get("labels")
        .and_then(Value::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(|l| l.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether `event` carries any one of `wanted` (an empty list keeps all).
fn labelled(event: &Event, wanted: &[String]) -> bool {
    wanted.is_empty() || labels_of(event).iter().any(|label| wanted.contains(label))
}

fn event_json(event: &Event) -> Value {
    json!({
        "id": event.id,
        "scope": event.scope,
        "modality": event.modality,
        "wal_offset": event.wal_offset,
        "content": event.content,
        "context": event.context,
    })
}

/// Lower-cased query words of 3+ alphanumeric characters.
fn words_of(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|word| word.chars().count() >= 3)
        .collect()
}

fn route(store: &mut Store, request: &Request) -> (u16, Value) {
    let route = request.path["/memory/".len()..].trim_end_matches('/');
    let body = &request.body;
    let method = request.method.as_str();
    match (method, route) {
        ("POST", "experience") => experience(store, request),
        ("GET", "events") => {
            let scope = request.query.get("scope").cloned().unwrap_or_default();
            let cursor: usize = request
                .query
                .get("cursor")
                .and_then(|c| c.parse().ok())
                .unwrap_or(0);
            let limit: usize = request
                .query
                .get("limit")
                .and_then(|l| l.parse().ok())
                .unwrap_or(50);
            let wanted: Vec<String> = request
                .query
                .get("labels")
                .map(|l| {
                    l.split(',')
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let mut stream = Vec::new();
            for event in store.events.iter().rev() {
                if event.scope == scope && labelled(event, &wanted) {
                    // The real listing emits every record twice.
                    stream.push(event_json(event));
                    stream.push(event_json(event));
                }
            }
            let items: Vec<Value> = stream.iter().skip(cursor).take(limit).cloned().collect();
            let next = cursor + items.len();
            ok(json!({
                "items": items,
                "has_more": next < stream.len(),
                "next_cursor": next.to_string(),
            }))
        }
        ("GET", id) if id.starts_with("events/") => {
            let id = &id["events/".len()..];
            match store.events.iter().find(|e| e.id == id) {
                Some(event) => ok(event_json(event)),
                None => fail(404, "NOT_FOUND"),
            }
        }
        ("POST", "recall") => {
            let scope = body["scope"].as_str().unwrap_or("").to_string();
            let descend = body["view"].as_str() == Some("descend");
            let wanted: Vec<String> = body
                .pointer("/filters/metadata/labels")
                .and_then(Value::as_array)
                .map(|l| {
                    l.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let words = words_of(body["query"].as_str().unwrap_or(""));
            let budget = body
                .pointer("/budgets/per_layer_limits/events")
                .and_then(Value::as_u64)
                .map_or(usize::MAX, |b| b as usize);
            let below = format!("{scope}/");
            let mut scored: Vec<(usize, Value)> = Vec::new();
            for event in store.events.iter().rev() {
                let in_scope = event.scope == scope || (descend && event.scope.starts_with(&below));
                if !in_scope || !labelled(event, &wanted) {
                    continue;
                }
                let text = text_of(&event.content).to_lowercase();
                let score = words.iter().filter(|w| text.contains(w.as_str())).count();
                if !words.is_empty() && score == 0 {
                    continue;
                }
                let mut rendered = event_json(event);
                let role = event.content["role"].as_str().unwrap_or("user").to_string();
                rendered["content"]["text"] =
                    json!(format!("[{role}] {}", text_of(&event.content)));
                scored.push((score, rendered));
            }
            scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
            let events: Vec<Value> = scored.into_iter().take(budget).map(|(_, e)| e).collect();
            store.next_pack += 1;
            let pack_id = format!("pack_{}", store.next_pack);
            store.packs.insert(pack_id.clone(), events.clone());
            ok(json!({ "pack_id": pack_id, "layers": { "events": events } }))
        }
        ("POST", "forget") => forget(store, body),
        ("GET", "scopes") => {
            let limit: usize = request
                .query
                .get("limit")
                .and_then(|l| l.parse().ok())
                .unwrap_or(50);
            let prefix = request.query.get("prefix").cloned().unwrap_or_default();
            let mut paths: Vec<String> = store
                .events
                .iter()
                .map(|e| e.scope.clone())
                .filter(|p| prefix.is_empty() || p.starts_with(&prefix))
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            paths.sort();
            let items: Vec<Value> = paths
                .into_iter()
                .take(limit)
                .map(|path| json!({ "path": path }))
                .collect();
            ok(json!({ "items": items }))
        }
        ("POST", "answer") => {
            let events = match body.get("use_pack_id").and_then(Value::as_str) {
                Some(id) => match store.packs.get(id) {
                    Some(pack) => pack.clone(),
                    None => return fail(400, "MISSING_PACK"),
                },
                None => Vec::new(),
            };
            let question = body["question"].as_str().unwrap_or("");
            let top = events.iter().map(|e| text_of(&e["content"])).next();
            let answer = match top {
                Some(text) => format!("grounded answer for {question}: {text}"),
                None => format!("grounded answer for {question}"),
            };
            let citations: Vec<Value> = events
                .iter()
                .take(5)
                .enumerate()
                .map(|(i, e)| {
                    json!({
                        "id": e["id"], "key": e["id"],
                        "content": text_of(&e["content"]),
                        "score": 1.0 / (1.0 + i as f64),
                    })
                })
                .collect();
            let context: Vec<String> = events.iter().map(|e| text_of(&e["content"])).collect();
            ok(json!({
                "answer": answer,
                "citations": citations,
                "context_block": context.join("\n"),
                "diagnostics": { "answer_model": "mock" },
            }))
        }
        _ => fail(404, "NOT_FOUND"),
    }
}

fn experience(store: &mut Store, request: &Request) -> (u16, Value) {
    let body = &request.body;
    let key = body["idempotency_key"].as_str().unwrap_or("").trim();
    if key.is_empty() {
        return fail(400, "MISSING_IDEMPOTENCY_KEY");
    }
    if let Some(claim) = &request.claim {
        // A retry of the same write reuses its header claim and must reach the
        // body-key replay; only a claim made for a different key conflicts.
        if store
            .claims
            .entry(claim.clone())
            .or_insert_with(|| key.to_string())
            != key
        {
            return fail(409, "CONFLICT");
        }
    }
    let scope = body["scope"].as_str().unwrap_or("").to_string();
    let modality = body["modality"].as_str().unwrap_or("").to_string();
    let content = body.get("content").cloned().unwrap_or_else(|| json!({}));
    let mut context = body
        .get("context")
        .filter(|c| c.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let fingerprint = format!("{scope}\u{0}{modality}\u{0}{content}\u{0}{context}");
    if let Some((seen, id)) = store.idempotency.get(key) {
        if *seen != fingerprint {
            return fail(409, "IDEMPOTENCY_CONFLICT");
        }
        return ok(json!({ "event_id": id, "replayed_from_idempotency": true }));
    }
    store.next_offset += 2;
    store.next_id += 1;
    let id = format!("evt_{}", store.next_id);
    store
        .idempotency
        .insert(key.to_string(), (fingerprint, id.clone()));
    context["recorded_at"] = json!("2026-01-01T00:00:00Z");
    let event = Event {
        id: id.clone(),
        scope,
        modality,
        wal_offset: store.next_offset,
        content,
        context,
    };
    store.events.push(event);
    ok(json!({ "event_id": id, "status": "captured", "replayed_from_idempotency": false }))
}

fn forget(store: &mut Store, body: &Value) -> (u16, Value) {
    let scope = body["scope"].as_str().unwrap_or("").to_string();
    let ids: Vec<String> = body
        .pointer("/selector/memory_ids")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let selective = !ids.is_empty();
    let confirm_all = body["confirm_all"].as_bool() == Some(true);
    if selective && confirm_all {
        return fail(400, "AMBIGUOUS_SELECTOR_CONFIRM_ALL");
    }
    if !selective && !confirm_all {
        return fail(422, "EMPTY_SELECTOR_WITHOUT_CONFIRMATION");
    }
    let cascade = body["cascade"].as_str().unwrap_or("derived_only");
    match cascade {
        "derived_only" => {
            return ok(json!({ "deleted": { "events": 0 }, "requested": ids.len(), "matched": 0 }));
        }
        "redact_events" => {}
        _ => return fail(400, "INVALID_CASCADE"),
    }
    let before = store.events.len();
    let removed: HashSet<String> = store
        .events
        .iter()
        .filter(|e| e.scope == scope && (!selective || ids.contains(&e.id)))
        .map(|e| e.id.clone())
        .collect();
    store.events.retain(|e| !removed.contains(&e.id));
    // A recall pack is derived data holding copies of events: drop every pack
    // that carries a forgotten one so `answer` cannot serve it afterwards.
    store.packs.retain(|_, pack| {
        !pack
            .iter()
            .any(|e| e["id"].as_str().is_some_and(|id| removed.contains(id)))
    });
    let deleted = before - store.events.len();
    ok(json!({ "deleted": { "events": deleted }, "requested": ids.len(), "matched": deleted }))
}
