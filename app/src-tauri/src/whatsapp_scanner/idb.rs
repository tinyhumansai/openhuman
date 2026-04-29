//! WhatsApp IndexedDB walk driven via the CDP `IndexedDB` domain.
//!
//! Replaces the old `scanner.js` in-page walk with pure CDP calls:
//!   * `IndexedDB.requestData` pages through each object store at the
//!     browser's C++ layer (no page-world JS needed to list rows).
//!   * `Runtime.callFunctionOn` with a fixed, WhatsApp-agnostic serializer
//!     (`function(){return [this].concat(arguments);}`) converts the
//!     resulting `Runtime.RemoteObject`s into JSON via `returnByValue`.
//!
//! The serializer is the only JS that executes in the page context. It is
//! structural — it can't read anything the page doesn't already hold — and
//! runs once per batch of ~100 records, not once per scan cycle. Records
//! are normalised in Rust (see `normalize_message` / `normalize_chat`).

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use super::CdpConn;

/// Only database that carries the chat + message stores. Discovered
/// empirically — a full `Target.getTargets` + `storeMap` dump (now removed)
/// showed every interesting store lives under `model-storage`.
const DATABASE_NAME: &str = "model-storage";
const MESSAGE_STORE: &str = "message";
const CHAT_STORE: &str = "chat";
const CONTACT_STORE: &str = "contact";
const GROUP_META_STORE: &str = "group-metadata";

/// Row window size per `IndexedDB.requestData` call. 500 keeps individual
/// CDP responses well under a megabyte while amortising request overhead.
const PAGE_SIZE: i64 = 500;
/// Hard cap per store. Mirrors the old JS limit so the full-scan cost
/// stays bounded on accounts with huge histories.
const MAX_RECORDS_PER_STORE: usize = 20_000;
/// How many RemoteObjects to materialise in one `Runtime.callFunctionOn`
/// batch. 100 keeps request argument counts reasonable and response bodies
/// in the low-MB range even for fat message records.
const SERIALIZE_BATCH: usize = 100;

/// Normalised message record — same shape the old `scanner.js` emitted so
/// the downstream merge / emit pipeline doesn't need to change. Bodies are
/// intentionally omitted: WhatsApp stores message text encrypted in IDB,
/// plaintext comes from the DOM snapshot path and is merged in by id.
#[derive(Debug, Clone, Default)]
pub struct IdbMessage {
    pub id: String,
    pub chat_id: String,
    pub from_me: bool,
    /// "me" for self-sent; otherwise the author/from JID.
    pub from: Option<String>,
    pub to: Option<String>,
    pub type_: Option<String>,
    pub timestamp: Option<i64>,
}

impl IdbMessage {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "chatId": self.chat_id,
            "fromMe": self.from_me,
            "from": self.from,
            "to": self.to,
            "type": self.type_,
            "timestamp": self.timestamp,
            // `body` deliberately absent — populated later by the DOM merge.
            "body": Value::Null,
        })
    }
}

/// Walk the WhatsApp IDB via CDP. Returns `(messages, chatNames)` where
/// `chatNames` is a `jid → display-name` map built from the chat, contact
/// and group-metadata stores. Per-store failures are logged and swallowed
/// so one unreadable store doesn't nuke the whole cycle.
pub async fn walk(
    cdp: &mut CdpConn,
    session: &str,
    url_prefix: &str,
) -> Result<(Vec<IdbMessage>, HashMap<String, String>), String> {
    let origin = origin_from_url(url_prefix)
        .ok_or_else(|| format!("cannot derive origin from {url_prefix}"))?;

    // `IndexedDB.enable` isn't strictly required for `requestData` on modern
    // Chromium but older CEF builds refuse without it. Cost is trivial.
    if let Err(e) = cdp.call("IndexedDB.enable", json!({}), Some(session)).await {
        log::debug!("[wa][idb] enable: {}", e);
    }

    let mut messages: Vec<IdbMessage> = Vec::new();
    let mut chat_names: HashMap<String, String> = HashMap::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    // Messages store → IdbMessage list, deduped by id.
    match read_store(cdp, session, &origin, MESSAGE_STORE).await {
        Ok(rows) => {
            for raw in &rows {
                if let Some(m) = normalize_message(raw) {
                    if seen_ids.insert(m.id.clone()) {
                        messages.push(m);
                    }
                }
            }
        }
        Err(e) => log::warn!("[wa][idb] read {} failed: {}", MESSAGE_STORE, e),
    }

    // Chat / contact / group-metadata stores → jid → name lookup. Last
    // write wins; the stores have disjoint id spaces in practice (contacts
    // use phone JIDs, groups use @g.us).
    for store in [CHAT_STORE, CONTACT_STORE, GROUP_META_STORE] {
        match read_store(cdp, session, &origin, store).await {
            Ok(rows) => {
                for raw in &rows {
                    let norm = if store == CONTACT_STORE {
                        normalize_contact(raw)
                    } else {
                        normalize_chat(raw)
                    };
                    if let Some((id, name)) = norm {
                        chat_names.insert(id, name);
                    }
                }
            }
            Err(e) => log::warn!("[wa][idb] read {} failed: {}", store, e),
        }
    }

    Ok((messages, chat_names))
}

// ─── CDP plumbing ───────────────────────────────────────────────────

/// Page through `objectStoreName` via `IndexedDB.requestData`, materialising
/// each value RemoteObject into JSON (via `serialize_values`). Stops at
/// `MAX_RECORDS_PER_STORE` or when `hasMore: false`.
async fn read_store(
    cdp: &mut CdpConn,
    session: &str,
    origin: &str,
    store: &str,
) -> Result<Vec<Value>, String> {
    let mut out: Vec<Value> = Vec::new();
    let mut skip: i64 = 0;
    loop {
        let remaining = MAX_RECORDS_PER_STORE.saturating_sub(out.len());
        if remaining == 0 {
            break;
        }
        let page = (remaining as i64).min(PAGE_SIZE);
        // NB: `indexName` is deliberately omitted — passing an empty
        // string makes this CEF build reject the request with
        // "Could not get index". The CDP spec says empty string means
        // "primary key index", but the C++ backend here only accepts an
        // unset field. Confirmed against CEF 146 (Chrome 146.0.7680.165).
        // Same fix as `slack_scanner/idb.rs` and `telegram_scanner/idb.rs`.
        let resp = cdp
            .call(
                "IndexedDB.requestData",
                json!({
                    "securityOrigin": origin,
                    "databaseName": DATABASE_NAME,
                    "objectStoreName": store,
                    "skipCount": skip,
                    "pageSize": page,
                }),
                Some(session),
            )
            .await?;
        let entries = resp
            .get("objectStoreDataEntries")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        if entries.is_empty() {
            break;
        }
        let value_refs: Vec<&Value> = entries
            .iter()
            .map(|e| e.get("value").unwrap_or(&Value::Null))
            .collect();
        let materialised = serialize_values(cdp, session, &value_refs).await?;
        out.extend(materialised);

        let has_more = resp
            .get("hasMore")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        skip += entries.len() as i64;
        if !has_more {
            break;
        }
    }
    log::debug!(
        "[wa][idb] store={} records={} (capped at {})",
        store,
        out.len(),
        MAX_RECORDS_PER_STORE
    );
    Ok(out)
}

/// Convert a list of `Runtime.RemoteObject` references (as returned inside
/// `ObjectStoreDataEntry.value`) into JSON. Primitives are read off the
/// RemoteObject's inline `value` field directly; complex objects are batched
/// through `Runtime.callFunctionOn` with a generic serializer.
async fn serialize_values(
    cdp: &mut CdpConn,
    session: &str,
    values: &[&Value],
) -> Result<Vec<Value>, String> {
    // Pre-split: inline primitives vs. objectIds that need serialization.
    // Keep positions so we can re-assemble in the original order.
    let mut result: Vec<Value> = vec![Value::Null; values.len()];
    let mut pending: Vec<(usize, String)> = Vec::new();
    for (i, v) in values.iter().enumerate() {
        // RemoteObject primitives carry their value inline.
        if let Some(inline) = v.get("value") {
            result[i] = inline.clone();
            continue;
        }
        if let Some(oid) = v.get("objectId").and_then(|x| x.as_str()) {
            pending.push((i, oid.to_string()));
            continue;
        }
        // Unserialisable RemoteObjects (e.g. `NaN`/`Infinity`) or ones
        // without an objectId get null — nothing downstream can use them.
    }
    for chunk in pending.chunks(SERIALIZE_BATCH) {
        let oids: Vec<&str> = chunk.iter().map(|(_, oid)| oid.as_str()).collect();
        let serialised = call_function_batch(cdp, session, &oids).await?;
        if serialised.len() != chunk.len() {
            return Err(format!(
                "serialise batch length mismatch: got {}, expected {}",
                serialised.len(),
                chunk.len()
            ));
        }
        for ((idx, _), val) in chunk.iter().zip(serialised.into_iter()) {
            result[*idx] = val;
        }
    }
    Ok(result)
}

/// Single `Runtime.callFunctionOn` invocation that materialises up to
/// `SERIALIZE_BATCH` RemoteObjects to JSON. The function body is fixed and
/// WhatsApp-agnostic — it just returns `[this, ...arguments]`. Uses the
/// first objectId as `this` (needed so Chromium knows which execution
/// context the call targets) and passes the rest as arguments.
async fn call_function_batch(
    cdp: &mut CdpConn,
    session: &str,
    object_ids: &[&str],
) -> Result<Vec<Value>, String> {
    if object_ids.is_empty() {
        return Ok(Vec::new());
    }
    let (first, rest) = object_ids.split_first().unwrap();
    let args: Vec<Value> = rest.iter().map(|oid| json!({ "objectId": oid })).collect();
    let resp = cdp
        .call(
            "Runtime.callFunctionOn",
            json!({
                "objectId": first,
                "functionDeclaration": "function(){return [this].concat(Array.prototype.slice.call(arguments));}",
                "arguments": args,
                "returnByValue": true,
                "silent": true,
            }),
            Some(session),
        )
        .await?;
    if let Some(exc) = resp.get("exceptionDetails") {
        return Err(format!("callFunctionOn threw: {exc}"));
    }
    let arr = resp
        .pointer("/result/value")
        .and_then(|v| v.as_array())
        .cloned()
        .ok_or_else(|| format!("callFunctionOn result not array: {resp}"))?;
    Ok(arr)
}

/// Parse `https://web.whatsapp.com/some/path` → `https://web.whatsapp.com`.
/// Returns `None` on URLs missing a scheme/host.
fn origin_from_url(u: &str) -> Option<String> {
    let (scheme, rest) = u.split_once("://")?;
    let host = rest.split('/').next()?;
    if scheme.is_empty() || host.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{host}"))
}

// ─── normalisation ──────────────────────────────────────────────────

/// WhatsApp's id fields take many shapes:
///   `"user@c.us"`,
///   `{_serialized: "user@c.us", …}`,
///   `{id: {_serialized: "..."}}`,
///   `{remote: {_serialized: "..."}}`.
/// Return the canonical JID string or None.
fn normalize_id(v: &Value) -> Option<String> {
    if v.is_null() {
        return None;
    }
    if let Some(s) = v.as_str() {
        return if s.is_empty() {
            None
        } else {
            Some(s.to_string())
        };
    }
    let obj = v.as_object()?;
    let str_of = |k: &str, src: &serde_json::Map<String, Value>| -> Option<String> {
        src.get(k)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    };
    if let Some(s) = str_of("_serialized", obj) {
        return Some(s);
    }
    if let Some(id) = obj.get("id") {
        if let Some(m) = id.as_object() {
            if let Some(s) = str_of("_serialized", m) {
                return Some(s);
            }
        }
        if let Some(s) = id.as_str() {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    if let Some(remote) = obj.get("remote") {
        if let Some(m) = remote.as_object() {
            if let Some(s) = str_of("_serialized", m) {
                return Some(s);
            }
        }
        if let Some(s) = remote.as_str() {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn normalize_message(raw: &Value) -> Option<IdbMessage> {
    let obj = raw.as_object()?;
    let id = obj
        .get("id")
        .and_then(normalize_id)
        .or_else(|| obj.get("_id").and_then(normalize_id))
        .or_else(|| obj.get("key").and_then(normalize_id))?;
    let from_jid = obj
        .get("from")
        .and_then(normalize_id)
        .or_else(|| obj.get("remoteJid").and_then(normalize_id));
    let to_jid = obj.get("to").and_then(normalize_id);
    let author = obj
        .get("author")
        .and_then(normalize_id)
        .or_else(|| obj.get("participant").and_then(normalize_id));
    let chat_id = obj
        .get("chatId")
        .and_then(normalize_id)
        .or_else(|| obj.get("remote").and_then(normalize_id))
        .or_else(|| from_jid.clone())
        .or_else(|| to_jid.clone())?;
    let from_me = obj.get("fromMe").and_then(|v| v.as_bool()).unwrap_or(false)
        || obj
            .get("isSentByMe")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        || obj
            .get("isFromMe")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let timestamp = obj
        .get("t")
        .and_then(|v| v.as_i64())
        .or_else(|| obj.get("timestamp").and_then(|v| v.as_i64()))
        .or_else(|| obj.get("messageTimestamp").and_then(|v| v.as_i64()));
    // `type` is usually the WA enum string; for raw-envelope records it
    // falls back to the first key of the `message` object (e.g.
    // `conversation`, `imageMessage`).
    let type_ = obj
        .get("type")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(|| {
            obj.get("message")
                .and_then(|m| m.as_object())
                .and_then(|m| m.keys().next().cloned())
        });
    let from = if from_me {
        Some("me".to_string())
    } else {
        author.or_else(|| from_jid.clone())
    };
    Some(IdbMessage {
        id,
        chat_id,
        from_me,
        from,
        to: to_jid,
        type_,
        timestamp,
    })
}

/// Chat / group records — `id` + first non-empty display name candidate.
fn normalize_chat(raw: &Value) -> Option<(String, String)> {
    let obj = raw.as_object()?;
    let id = obj
        .get("id")
        .and_then(normalize_id)
        .or_else(|| obj.get("_id").and_then(normalize_id))?;
    let name = first_non_empty_str(obj, &["name", "subject", "formattedTitle"]).or_else(|| {
        obj.get("contact")
            .and_then(|c| c.as_object())
            .and_then(|c| first_non_empty_str(c, &["name", "pushname"]))
    })?;
    Some((id, name))
}

/// Contact records — different name priority from chat records (contacts
/// carry `notify`/`pushname`/`verifiedName` in addition to the usual).
fn normalize_contact(raw: &Value) -> Option<(String, String)> {
    let obj = raw.as_object()?;
    let id = obj
        .get("id")
        .and_then(normalize_id)
        .or_else(|| obj.get("_id").and_then(normalize_id))?;
    let name = first_non_empty_str(
        obj,
        &["name", "notify", "shortName", "pushname", "verifiedName"],
    )?;
    Some((id, name))
}

fn first_non_empty_str(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = obj.get(*k).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "idb_tests.rs"]
mod tests;
