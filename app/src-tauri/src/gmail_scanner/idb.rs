//! Gmail IndexedDB backfill driven purely through the CDP `IndexedDB` domain.
//!
//! Mirrors `slack_scanner::idb` — no JavaScript runs in the page. We use:
//!   - `IndexedDB.requestDatabaseNames` to enumerate databases at the Gmail
//!     security origin.
//!   - `IndexedDB.requestDatabase` to list object stores.
//!   - `IndexedDB.requestData` paged to walk records.
//!
//! We filter to Gmail's offline-cache databases:
//!   - `ItemMail_<email>_<suffix>` — per-message objects.
//!   - `SyncData_<email>_<suffix>` — sync state / metadata.
//!
//! Each record is emitted as a `webview:event` envelope with
//! `payload.source: "cdp-idb-record"`. The JS layer (or the core RPC
//! handler wired up via `GmailPanel.tsx`) normalises records and calls
//! `gmail.ingest_raw_response`.

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Runtime};

use super::{browser_ws_url, now_millis, CdpConn};

/// CDP security origin for Gmail web.
const GMAIL_ORIGIN: &str = "https://mail.google.com";

/// Number of records per `IndexedDB.requestData` page.
const PAGE_SIZE: i64 = 100;

/// Per-store safety ceiling to guard against runaway object stores.
const MAX_RECORDS_PER_STORE: usize = 10_000;

/// Prefixes of Gmail offline-cache databases we care about.
const GMAIL_DB_PREFIXES: &[&str] = &["ItemMail_", "SyncData_"];

/// Database names we can safely skip.
const SKIP_DB_PREFIXES: &[&str] = &["databases"]; // Chromium metadata

// ---------------------------------------------------------------------------
// Public entry-point
// ---------------------------------------------------------------------------

/// Open a fresh CDP connection, enumerate Gmail's IndexedDB databases,
/// page through records, and emit each as a `webview:event` with
/// `source: "cdp-idb-record"`. Returns the total record count emitted.
///
/// This function opens its own connection and does NOT share the live MITM
/// session — backfill is a one-shot operation.
pub async fn backfill<R: Runtime>(app: &AppHandle<R>, account_id: &str) -> Result<usize, String> {
    log::info!(
        "[gmail][{}][idb] backfill start origin={}",
        account_id,
        GMAIL_ORIGIN
    );

    let browser_ws = browser_ws_url().await?;
    let mut cdp = CdpConn::open(&browser_ws).await?;

    // Attach to the Gmail page target so we can reach its IDB storage.
    let targets_v = cdp.call("Target.getTargets", json!({}), None).await?;
    let page = {
        let arr = targets_v
            .get("targetInfos")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        arr.into_iter()
            .find(|t| {
                t.get("type").and_then(|v| v.as_str()) == Some("page")
                    && t.get("url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .starts_with("https://mail.google.com/")
            })
            .ok_or_else(|| "no Gmail page target found for IDB backfill".to_string())?
    };

    let page_id = page
        .get("targetId")
        .and_then(|v| v.as_str())
        .ok_or("missing targetId")?
        .to_string();
    log::debug!(
        "[gmail][{}][idb] attaching to page_id={}",
        account_id,
        page_id
    );

    let attach = cdp
        .call(
            "Target.attachToTarget",
            json!({ "targetId": page_id, "flatten": true }),
            None,
        )
        .await?;
    let session = attach
        .get("sessionId")
        .and_then(|x| x.as_str())
        .ok_or("attach missing sessionId")?
        .to_string();

    // Enable IndexedDB domain.
    if let Err(e) = cdp
        .call("IndexedDB.enable", json!({}), Some(&session))
        .await
    {
        log::debug!("[gmail][{}][idb] IndexedDB.enable: {}", account_id, e);
    }

    // Enumerate database names at the Gmail origin.
    let names_v = cdp
        .call(
            "IndexedDB.requestDatabaseNames",
            json!({ "securityOrigin": GMAIL_ORIGIN }),
            Some(&session),
        )
        .await?;
    let all_names: Vec<String> = names_v
        .get("databaseNames")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    log::info!(
        "[gmail][{}][idb] found {} databases at origin={}",
        account_id,
        all_names.len(),
        GMAIL_ORIGIN
    );

    // Filter to Gmail mail/sync databases.
    let gmail_names: Vec<String> = all_names
        .into_iter()
        .filter(|name| {
            if SKIP_DB_PREFIXES.iter().any(|p| name.starts_with(p)) {
                return false;
            }
            GMAIL_DB_PREFIXES.iter().any(|p| name.starts_with(p))
        })
        .collect();

    log::info!(
        "[gmail][{}][idb] {} Gmail databases to scan: {:?}",
        account_id,
        gmail_names.len(),
        gmail_names
    );

    let mut total = 0usize;
    for db_name in &gmail_names {
        match walk_database(&mut cdp, &session, db_name, app, account_id).await {
            Ok(n) => {
                log::info!(
                    "[gmail][{}][idb] db={} records_emitted={}",
                    account_id,
                    db_name,
                    n
                );
                total += n;
            }
            Err(e) => {
                log::warn!("[gmail][{}][idb] db={} failed: {}", account_id, db_name, e);
            }
        }
    }

    // Detach cleanly (best-effort).
    let _ = cdp
        .call(
            "Target.detachFromTarget",
            json!({ "sessionId": session }),
            None,
        )
        .await;

    log::info!(
        "[gmail][{}][idb] backfill done total_records={}",
        account_id,
        total
    );
    Ok(total)
}

// ---------------------------------------------------------------------------
// Per-database walk
// ---------------------------------------------------------------------------

async fn walk_database<R: Runtime>(
    cdp: &mut CdpConn,
    session: &str,
    db_name: &str,
    app: &AppHandle<R>,
    account_id: &str,
) -> Result<usize, String> {
    let meta = cdp
        .call(
            "IndexedDB.requestDatabase",
            json!({
                "securityOrigin": GMAIL_ORIGIN,
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

    log::debug!(
        "[gmail][{}][idb] db={} stores={:?}",
        account_id,
        db_name,
        store_names
    );

    let mut total = 0usize;
    for store_name in &store_names {
        match read_store(cdp, session, db_name, store_name, app, account_id).await {
            Ok(n) => {
                log::debug!(
                    "[gmail][{}][idb] db={} store={} emitted={}",
                    account_id,
                    db_name,
                    store_name,
                    n
                );
                total += n;
            }
            Err(e) => {
                log::warn!(
                    "[gmail][{}][idb] db={} store={} failed: {}",
                    account_id,
                    db_name,
                    store_name,
                    e
                );
            }
        }
    }
    Ok(total)
}

// ---------------------------------------------------------------------------
// Per-store page walk
// ---------------------------------------------------------------------------

async fn read_store<R: Runtime>(
    cdp: &mut CdpConn,
    session: &str,
    database_name: &str,
    store: &str,
    app: &AppHandle<R>,
    account_id: &str,
) -> Result<usize, String> {
    let mut skip: i64 = 0;
    let mut emitted = 0usize;

    loop {
        let remaining = MAX_RECORDS_PER_STORE.saturating_sub(emitted);
        if remaining == 0 {
            log::debug!(
                "[gmail][{}][idb] store={} hit MAX_RECORDS cap, stopping",
                account_id,
                store
            );
            break;
        }
        let page = (remaining as i64).min(PAGE_SIZE);

        // NB: `indexName` is omitted intentionally — see slack_scanner/idb.rs
        // comment about CEF 146 rejecting empty-string indexName.
        let resp = cdp
            .call_with_timeout(
                "IndexedDB.requestData",
                json!({
                    "securityOrigin": GMAIL_ORIGIN,
                    "databaseName": database_name,
                    "objectStoreName": store,
                    "skipCount": skip,
                    "pageSize": page,
                }),
                Some(session),
                std::time::Duration::from_secs(60),
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

        let batch_size = entries.len();
        skip += batch_size as i64;

        // Emit each record as a webview:event. We don't materialise
        // RemoteObjects here (no `Runtime.callFunctionOn`) because Gmail's
        // IDB records come back as inline JSON values in most Chrome builds,
        // and the consumer (GmailPanel forwarder → core) is responsible for
        // normalisation. For now we emit the raw entry Value.
        for entry in &entries {
            let value = entry.get("value").cloned().unwrap_or(Value::Null);
            emit_idb_record(app, account_id, database_name, store, value);
            emitted += 1;
        }

        let has_more = resp
            .get("hasMore")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        if !has_more {
            break;
        }
    }

    Ok(emitted)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn emit_idb_record<R: Runtime>(
    app: &AppHandle<R>,
    account_id: &str,
    db_name: &str,
    store_name: &str,
    record: Value,
) {
    let envelope = json!({
        "account_id": account_id,
        "provider": "gmail",
        "kind": "ingest",
        "payload": {
            "provider": "gmail",
            "source": "cdp-idb-record",
            "db_name": db_name,
            "store_name": store_name,
            "record": record,
        },
        "ts": now_millis(),
    });
    if let Err(e) = app.emit("webview:event", &envelope) {
        log::warn!("[gmail][{}][idb] emit failed: {}", account_id, e);
    }
}
