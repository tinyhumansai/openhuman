//! CDP-driven walk of the Google Messages Web `bugle_db` IndexedDB.
//!
//! Pairs with `idb.rs` (schema + normalization). This module does the
//! `IndexedDB.requestData` paging + `Runtime.callFunctionOn` serialisation
//! dance, then hands the raw JSON rows to `idb::normalize_*` for shape
//! checking.

use serde_json::{json, Value};

use super::idb::{
    self, Conversation, Message, ParticipantMap, DATABASE_NAME, STORE_CONVERSATIONS,
    STORE_MESSAGES, STORE_PARTICIPANTS,
};
use crate::cdp::CdpConn;

/// IndexedDB security origin for the Google Messages Web app.
const ORIGIN: &str = "https://messages.google.com";
/// Rows per `IndexedDB.requestData` call — matches the WhatsApp scanner.
const PAGE_SIZE: i64 = 500;
/// Hard cap per store to bound full-scan cost on huge histories.
const MAX_RECORDS_PER_STORE: usize = 20_000;
/// `Runtime.callFunctionOn` batch size for RemoteObject serialisation.
const SERIALIZE_BATCH: usize = 100;

pub struct WalkResult {
    pub messages: Vec<Message>,
    pub conversations: Vec<Conversation>,
    pub participants: ParticipantMap,
}

/// Walk `bugle_db`: messages, conversations, participants. Per-store
/// failures are logged and swallowed so one bad store doesn't nuke the
/// cycle — the caller still gets whatever did come back.
pub async fn walk(cdp: &mut CdpConn, session: &str) -> Result<WalkResult, String> {
    // `IndexedDB.enable` is a no-op on modern Chromium but older CEF
    // builds refuse `requestData` without it. Cost is trivial.
    if let Err(e) = cdp.call("IndexedDB.enable", json!({}), Some(session)).await {
        log::debug!("[gmessages][idb] enable: {}", e);
    }

    let messages_raw = match read_store(cdp, session, STORE_MESSAGES).await {
        Ok(v) => v,
        Err(e) => {
            log::warn!("[gmessages][idb] read {} failed: {}", STORE_MESSAGES, e);
            Vec::new()
        }
    };
    let convos_raw = match read_store(cdp, session, STORE_CONVERSATIONS).await {
        Ok(v) => v,
        Err(e) => {
            log::warn!("[gmessages][idb] read {} failed: {}", STORE_CONVERSATIONS, e);
            Vec::new()
        }
    };
    let parts_raw = match read_store(cdp, session, STORE_PARTICIPANTS).await {
        Ok(v) => v,
        Err(e) => {
            log::warn!("[gmessages][idb] read {} failed: {}", STORE_PARTICIPANTS, e);
            Vec::new()
        }
    };

    let messages: Vec<Message> = messages_raw
        .iter()
        .filter_map(idb::normalize_message)
        .collect();
    let conversations: Vec<Conversation> = convos_raw
        .iter()
        .filter_map(idb::normalize_conversation)
        .collect();
    let mut participants = ParticipantMap::default();
    for raw in &parts_raw {
        if let Some((id, name)) = idb::normalize_participant(raw) {
            participants.insert(id, name);
        }
    }

    log::info!(
        "[gmessages][idb] walk messages={} conversations={} participants={}",
        messages.len(),
        conversations.len(),
        participants.len()
    );
    Ok(WalkResult {
        messages,
        conversations,
        participants,
    })
}

async fn read_store(cdp: &mut CdpConn, session: &str, store: &str) -> Result<Vec<Value>, String> {
    let mut out: Vec<Value> = Vec::new();
    let mut skip: i64 = 0;
    loop {
        let remaining = MAX_RECORDS_PER_STORE.saturating_sub(out.len());
        if remaining == 0 {
            break;
        }
        let page = (remaining as i64).min(PAGE_SIZE);
        let resp = cdp
            .call(
                "IndexedDB.requestData",
                json!({
                    "securityOrigin": ORIGIN,
                    "databaseName": DATABASE_NAME,
                    "objectStoreName": store,
                    "indexName": "",
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
    log::debug!("[gmessages][idb] store={} records={}", store, out.len());
    Ok(out)
}

async fn serialize_values(
    cdp: &mut CdpConn,
    session: &str,
    values: &[&Value],
) -> Result<Vec<Value>, String> {
    let mut result: Vec<Value> = vec![Value::Null; values.len()];
    let mut pending: Vec<(usize, String)> = Vec::new();
    for (i, v) in values.iter().enumerate() {
        if let Some(inline) = v.get("value") {
            result[i] = inline.clone();
            continue;
        }
        if let Some(oid) = v.get("objectId").and_then(|x| x.as_str()) {
            pending.push((i, oid.to_string()));
        }
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
    resp.pointer("/result/value")
        .and_then(|v| v.as_array())
        .cloned()
        .ok_or_else(|| format!("callFunctionOn result not array: {resp}"))
}
