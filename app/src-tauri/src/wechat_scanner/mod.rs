//! WeChat Web scanner over CDP — chat list + active conversation DOM scrape.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use openhuman_core::openhuman::webview_accounts::{
    list_ingest_envelope, memory_doc_ingest_list_snapshot, memory_doc_ingest_peer_transcript,
    validate_scan, WechatMessageRow, WechatScanPayload,
};
use parking_lot::Mutex;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Runtime};
use tokio::task::AbortHandle;
use tokio::time::sleep;

mod dom_snapshot;

const SCAN_INTERVAL: Duration = Duration::from_secs(3);
const STARTUP_DELAY: Duration = Duration::from_secs(8);

pub fn wechat_scanner_disabled() -> bool {
    matches!(
        std::env::var("OPENHUMAN_DISABLE_WECHAT_SCANNER")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("1") | Some("true") | Some("yes")
    )
}

pub fn spawn_scanner<R: Runtime>(
    app: AppHandle<R>,
    account_id: String,
    url_prefix: String,
) -> AbortHandle {
    tokio::spawn(async move {
        let fragment = crate::cdp::target_url_fragment(&account_id);
        log::info!(
            "[wechat] scanner up account={} url_prefix={} fragment={}",
            account_id,
            url_prefix,
            fragment
        );
        sleep(STARTUP_DELAY).await;
        let mut last_hash: Option<u64> = None;
        loop {
            match scan_once(&url_prefix, &fragment).await {
                Ok(scan) => {
                    if Some(scan.hash) == last_hash {
                        sleep(SCAN_INTERVAL).await;
                        continue;
                    }
                    last_hash = Some(scan.hash);
                    let payload = dom_snapshot::scan_to_core_payload(&account_id, &scan);
                    if validate_scan(&payload).is_err() {
                        sleep(SCAN_INTERVAL).await;
                        continue;
                    }
                    log::info!(
                        "[wechat][{}] dom scan chats={} msgs={} unread={}",
                        account_id,
                        scan.chat_rows.len(),
                        scan.messages.len(),
                        scan.unread
                    );
                    emit_and_persist(&app, &account_id, &payload);
                }
                Err(e) => log::debug!("[wechat][{}] dom scan failed: {}", account_id, e),
            }
            sleep(SCAN_INTERVAL).await;
        }
    })
    .abort_handle()
}

async fn scan_once(url_prefix: &str, url_fragment: &str) -> Result<dom_snapshot::DomScan, String> {
    let prefix = url_prefix.to_string();
    let fragment = url_fragment.to_string();
    let (mut cdp, session) = crate::cdp::connect_and_attach_matching(move |t| {
        t.url.starts_with(&prefix) && t.url.ends_with(&fragment)
    })
    .await?;
    let scan = dom_snapshot::scan(&mut cdp, &session).await;
    crate::cdp::detach_session(&mut cdp, &session).await;
    scan
}

fn emit_and_persist<R: Runtime>(app: &AppHandle<R>, account_id: &str, payload: &WechatScanPayload) {
    if let Err(e) = app.emit(
        "webview:event",
        &list_ingest_envelope(account_id, payload, chrono_now_millis()),
    ) {
        log::warn!("[wechat][{}] ingest emit failed: {}", account_id, e);
    }
    if !payload.chat_rows.is_empty() {
        let acct = account_id.to_string();
        let list = payload.clone();
        tokio::spawn(async move {
            if let Err(e) = post_memory_doc(&acct, memory_doc_ingest_list_snapshot(&list)).await {
                log::warn!("[wechat][{}] list memory failed: {}", acct, e);
            }
        });
    }
    let mut groups: HashMap<String, (String, Vec<WechatMessageRow>)> = HashMap::new();
    for m in &payload.messages {
        if m.body.trim().is_empty() {
            continue;
        }
        let e = groups.entry(m.chat_id.clone()).or_default();
        if e.0.is_empty() {
            e.0 = m.chat_name.clone();
        }
        e.1.push(m.clone());
    }
    for (chat_id, (chat_name, rows)) in groups {
        let acct = account_id.to_string();
        tokio::spawn(async move {
            match memory_doc_ingest_peer_transcript(&acct, &chat_id, &chat_name, &rows) {
                Ok(params) => {
                    if let Err(e) = post_memory_doc(&acct, Ok(params)).await {
                        log::warn!(
                            "[wechat][{}] peer memory upsert failed chat_id={}: {}",
                            acct,
                            chat_id,
                            e
                        );
                    }
                }
                Err(e) => log::warn!(
                    "[wechat][{}] peer transcript build failed chat_id={}: {}",
                    acct,
                    chat_id,
                    e
                ),
            }
        });
    }
}

async fn post_memory_doc(
    account_id: &str,
    params: Result<serde_json::Map<String, Value>, String>,
) -> Result<(), String> {
    let params = params?;
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "openhuman.memory_doc_ingest",
        "params": Value::Object(params),
    });
    let url = crate::core_rpc::core_rpc_url_value();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let resp = crate::core_rpc::apply_auth(client.post(&url))
        .map_err(|e| format!("prepare {url}: {e}"))?
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("POST {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "{}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        ));
    }
    let v: Value = resp.json().await.map_err(|e| format!("decode: {e}"))?;
    if v.get("error").is_some() {
        return Err(format!("rpc error: {}", v["error"]));
    }
    log::info!("[wechat][{}] memory upsert ok", account_id);
    Ok(())
}

fn chrono_now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Default)]
pub struct ScannerRegistry {
    started: Mutex<HashMap<String, AbortHandle>>,
}

impl ScannerRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn ensure_scanner<R: Runtime>(
        &self,
        app: AppHandle<R>,
        account_id: String,
        url_prefix: String,
    ) {
        if wechat_scanner_disabled() {
            return;
        }
        let mut g = self.started.lock();
        if g.contains_key(&account_id) {
            return;
        }
        let scanner_account_id = account_id.clone();
        g.insert(
            account_id,
            spawn_scanner(app, scanner_account_id, url_prefix),
        );
    }

    pub fn forget(&self, account_id: &str) {
        if let Some(h) = self.started.lock().remove(account_id) {
            h.abort();
        }
    }

    pub fn forget_all(&self) -> usize {
        let entries: Vec<_> = self.started.lock().drain().collect();
        for (_, h) in &entries {
            h.abort();
        }
        entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_env_var_is_honored() {
        std::env::set_var("OPENHUMAN_DISABLE_WECHAT_SCANNER", "1");
        assert!(wechat_scanner_disabled());
        std::env::remove_var("OPENHUMAN_DISABLE_WECHAT_SCANNER");
    }
}
