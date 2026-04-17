//! Slack IndexedDB walk driven purely through the CDP `IndexedDB` domain.
//!
//! No JavaScript runs in the page — `IndexedDB.requestDatabaseNames`,
//! `IndexedDB.requestDatabase`, and `IndexedDB.requestData` page through
//! every store at the browser's C++ layer. `Runtime.callFunctionOn` with a
//! fixed, Slack-agnostic serializer (`function(){return [this].concat(arguments);}`)
//! turns each batch of `Runtime.RemoteObject`s into JSON via `returnByValue`.
//! The serializer is structural; it can't read anything the page doesn't
//! already hold. It runs once per batch of ~100 records, not once per scan.
//!
//! Slack persists its Redux state tree to a database named
//! `ReduxPersistIDB:<team_id>_<user_id>` with a single object store
//! (`state`) whose records are redux-persist snapshots. We also pick up
//! other Slack-owned databases (session, calls, etc.) opportunistically.
//!
//! Harvested JSON is walked recursively in `extract` to pull message-,
//! user-, and channel-shaped records. Unlike WhatsApp we can't hit a
//! single known (database, store) pair because Slack namespaces DBs per
//! workspace and the actual schema has moved across Slack versions —
//! enumeration is cheap and gives us future-proofing for free.

use serde_json::{json, Value};

use super::CdpConn;

/// CDP-known origin for the Slack web app.
const ORIGIN: &str = "https://app.slack.com";
/// Row window per `IndexedDB.requestData` call. Slack's individual Redux
/// snapshot records can be multi-megabyte, so we keep the page small.
const PAGE_SIZE: i64 = 50;
/// Per-store ceiling. Slack workspaces can legitimately exceed this; the
/// cap is a safety net against runaway stores, not a hard limit.
const MAX_RECORDS_PER_STORE: usize = 5_000;
/// Max `Runtime.RemoteObject`s to materialise in a single
/// `Runtime.callFunctionOn`. Smaller than WhatsApp's 100 because each
/// Slack record can carry dozens of KB.
const SERIALIZE_BATCH: usize = 32;
/// Skip databases we know aren't useful (and would waste scan budget).
const SKIP_DB_PREFIXES: &[&str] = &[
    "SlackDesktopSettings",
    "webpack",
    "databases", // Chromium's own metadata DB
];

/// Product of one full walk — raw records grouped by (database, store)
/// so downstream extraction can log per-source counts. Debug-only fields
/// (`error`, `count`, `name`) are kept for log/inspection even though the
/// extractor only reads `records`.
#[derive(Debug, Default)]
pub struct IdbDump {
    pub dbs: Vec<IdbDb>,
}

#[derive(Debug, Default)]
#[allow(dead_code)]
pub struct IdbDb {
    pub name: String,
    pub stores: Vec<IdbStore>,
    pub error: Option<String>,
}

#[derive(Debug, Default)]
#[allow(dead_code)]
pub struct IdbStore {
    pub name: String,
    pub records: Vec<Value>,
    pub count: i64,
    pub error: Option<String>,
}

/// Walk every Slack-relevant IndexedDB database on `ORIGIN`. Returns a
/// flat dump — no per-record normalisation happens here; that lives in
/// `extract::walk_extract` because the record shapes vary across stores.
pub async fn walk(cdp: &mut CdpConn, session: &str) -> Result<IdbDump, String> {
    if let Err(e) = cdp.call("IndexedDB.enable", json!({}), Some(session)).await {
        log::debug!("[sl][idb] enable: {}", e);
    }

    let names_v = cdp
        .call(
            "IndexedDB.requestDatabaseNames",
            json!({ "securityOrigin": ORIGIN }),
            Some(session),
        )
        .await?;
    let names: Vec<String> = names_v
        .get("databaseNames")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    log::info!(
        "[sl][idb] found {} databases at origin {}: {:?}",
        names.len(),
        ORIGIN,
        names
    );

    let mut dump = IdbDump::default();
    for name in names {
        if SKIP_DB_PREFIXES.iter().any(|p| name.starts_with(p)) {
            log::debug!("[sl][idb] skipping db {}", name);
            continue;
        }
        match walk_database(cdp, session, &name).await {
            Ok(db) => {
                log::info!(
                    "[sl][idb] db={} stores={} total_records={}",
                    db.name,
                    db.stores.len(),
                    db.stores.iter().map(|s| s.records.len()).sum::<usize>()
                );
                dump.dbs.push(db);
            }
            Err(e) => {
                log::warn!("[sl][idb] db={} failed: {}", name, e);
                dump.dbs.push(IdbDb {
                    name,
                    error: Some(e),
                    ..Default::default()
                });
            }
        }
    }
    Ok(dump)
}

async fn walk_database(cdp: &mut CdpConn, session: &str, db_name: &str) -> Result<IdbDb, String> {
    let meta = cdp
        .call(
            "IndexedDB.requestDatabase",
            json!({
                "securityOrigin": ORIGIN,
                "databaseName": db_name,
            }),
            Some(session),
        )
        .await?;

    let store_names: Vec<String> = meta
        .pointer("/databaseWithObjectStores/objectStores")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut db = IdbDb {
        name: db_name.to_string(),
        ..Default::default()
    };
    for store_name in store_names {
        match read_store(cdp, session, db_name, &store_name).await {
            Ok((records, count)) => {
                log::debug!(
                    "[sl][idb] db={} store={} count={} fetched={}",
                    db_name,
                    store_name,
                    count,
                    records.len()
                );
                db.stores.push(IdbStore {
                    name: store_name,
                    records,
                    count,
                    error: None,
                });
            }
            Err(e) => {
                log::warn!(
                    "[sl][idb] db={} store={} failed: {}",
                    db_name,
                    store_name,
                    e
                );
                db.stores.push(IdbStore {
                    name: store_name,
                    error: Some(e),
                    ..Default::default()
                });
            }
        }
    }
    Ok(db)
}

/// Page through `objectStoreName` via `IndexedDB.requestData`, materialising
/// each value RemoteObject into JSON. Stops at `MAX_RECORDS_PER_STORE` or
/// when `hasMore: false`. Returns `(records, total_fetched_count)`.
async fn read_store(
    cdp: &mut CdpConn,
    session: &str,
    database_name: &str,
    store: &str,
) -> Result<(Vec<Value>, i64), String> {
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
        let resp = cdp
            .call(
                "IndexedDB.requestData",
                json!({
                    "securityOrigin": ORIGIN,
                    "databaseName": database_name,
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
    Ok((out, skip))
}

/// Convert a list of `Runtime.RemoteObject` references (as returned inside
/// `ObjectStoreDataEntry.value`) into JSON. Primitives are read off the
/// RemoteObject's inline `value`; complex objects are batched through
/// `Runtime.callFunctionOn` with a generic serializer. Same pattern as
/// `whatsapp_scanner::idb::serialize_values`.
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
            continue;
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
        .call_with_timeout(
            "Runtime.callFunctionOn",
            json!({
                "objectId": first,
                "functionDeclaration": "function(){return [this].concat(Array.prototype.slice.call(arguments));}",
                "arguments": args,
                "returnByValue": true,
                "silent": true,
            }),
            Some(session),
            std::time::Duration::from_secs(60),
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
