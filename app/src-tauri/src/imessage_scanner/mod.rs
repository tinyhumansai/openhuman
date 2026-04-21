//! iMessage local-database scanner.
//!
//! Reads `~/Library/Messages/chat.db` on macOS (read-only) and emits one
//! `openhuman.memory_doc_ingest` JSON-RPC call per `(chat_identifier, day)`
//! group — matching the convention codified in
//! `docs/webview-integration-playbook.md` and used by the WhatsApp scanner.
//!
//! Unlike the webview scanners this needs no CEF / CDP / DOM / IDB — iMessage
//! persists everything in a local SQLite file. One tick is enough; no
//! fast/full split.
//!
//! macOS-only. On other platforms the scanner is a no-op.

#[cfg(target_os = "macos")]
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::sync::{Arc, OnceLock};
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "macos")]
use parking_lot::Mutex;
#[cfg(target_os = "macos")]
use serde_json::json;
#[cfg(target_os = "macos")]
use tauri::{AppHandle, Runtime};
#[cfg(target_os = "macos")]
use tokio::time::sleep;

/// Shared HTTP client reused across scanner ticks. `reqwest::Client` holds a
/// connection pool and bundles rustls roots at construction — creating one
/// per ingest call burns CPU and fragments keep-alive reuse.
#[cfg(target_os = "macos")]
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

#[cfg(target_os = "macos")]
fn http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// Cap on rows read for a single day's rebuild — chat.db one-day slice is
/// almost always tiny, but we guard against pathological group chats.
#[cfg(target_os = "macos")]
const MAX_MESSAGES_PER_DAY_REBUILD: usize = 5000;

#[cfg(target_os = "macos")]
mod chatdb;

#[cfg(target_os = "macos")]
const SCAN_INTERVAL: Duration = Duration::from_secs(60);
#[cfg(target_os = "macos")]
const MAX_MESSAGES_PER_TICK: usize = 2000;

/// Registry tracking one scanner per "account". iMessage effectively has one
/// account per macOS user, but we keep the registry shape symmetric with
/// the webview scanners for future multi-account support.
#[cfg(target_os = "macos")]
pub struct ScannerRegistry {
    inner: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

#[cfg(target_os = "macos")]
impl ScannerRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Spawn the scanner loop if not already running. Idempotent.
    pub fn ensure_scanner<R: Runtime>(self: Arc<Self>, app: AppHandle<R>, account_id: String) {
        let mut guard = self.inner.lock();
        if guard.as_ref().map_or(false, |h| !h.is_finished()) {
            return;
        }
        let handle = tokio::spawn(run_scanner(app, account_id));
        *guard = Some(handle);
    }
}

#[cfg(target_os = "macos")]
async fn run_scanner<R: Runtime>(app: AppHandle<R>, account_id: String) {
    log::info!(
        "[imessage] scanner up account={} interval={:?}",
        account_id,
        SCAN_INTERVAL
    );

    let db_path = match chat_db_path() {
        Some(p) => p,
        None => {
            log::warn!("[imessage] cannot resolve chat.db path — scanner exiting");
            return;
        }
    };

    // Restore cursor from disk so a crash/restart doesn't re-ingest history.
    let cursor_path = cursor_file_path(&app, &account_id);
    let mut last_rowid: i64 = read_cursor(&cursor_path).unwrap_or(0);
    log::info!(
        "[imessage][{}] cursor restored rowid={} path={:?}",
        account_id,
        last_rowid,
        cursor_path
    );

    loop {
        // Gate every tick on explicit iMessage connection state: no ingestion
        // before opt-in, and stops immediately when the user disconnects.
        let gate = match fetch_imessage_gate().await {
            Ok(g) => g,
            Err(e) => {
                log::debug!("[imessage] config fetch failed (will retry): {}", e);
                sleep(SCAN_INTERVAL).await;
                continue;
            }
        };
        let Some(allowed_contacts) = gate else {
            log::debug!("[imessage] not connected — skipping tick");
            sleep(SCAN_INTERVAL).await;
            continue;
        };

        match chatdb::read_since(&db_path, last_rowid, MAX_MESSAGES_PER_TICK) {
            Ok(messages) if messages.is_empty() => {
                log::debug!("[imessage] no new messages since rowid={}", last_rowid);
            }
            Ok(messages) => {
                // Remember max rowid we observed in THIS tick so the cursor
                // can advance if all groups ingest successfully.
                let tick_max_rowid = messages.iter().map(|m| m.rowid).max().unwrap_or(last_rowid);

                // Collect unique (chat_identifier, apple_ns_within_day)
                // pairs touched by this tick — we rebuild the full day for
                // each before upserting, so per-tick POSTs contain the
                // complete conversation and not just the delta.
                let day_keys = unique_chat_day_keys(&messages);
                log::info!(
                    "[imessage][{}] scan ok new_rows={} unique_days={} cursor={}",
                    account_id,
                    messages.len(),
                    day_keys.len(),
                    tick_max_rowid
                );

                let mut had_group_failure = false;
                for (chat_id, anchor_secs) in day_keys {
                    if !chat_allowed(&chat_id, &allowed_contacts) {
                        log::debug!(
                            "[imessage] skipping chat={} — not in allowed_contacts",
                            chat_id
                        );
                        continue;
                    }
                    let (start_ns, end_ns) = local_day_bounds_apple_ns(anchor_secs);
                    let full_day = match chatdb::read_chat_day(
                        &db_path,
                        &chat_id,
                        start_ns,
                        end_ns,
                        MAX_MESSAGES_PER_DAY_REBUILD,
                    ) {
                        Ok(msgs) => msgs,
                        Err(e) => {
                            log::warn!(
                                "[imessage] full-day read failed chat={} err={}",
                                chat_id,
                                e
                            );
                            had_group_failure = true;
                            continue;
                        }
                    };
                    if full_day.is_empty() {
                        continue;
                    }
                    let day_ymd = seconds_to_ymd(anchor_secs);
                    let key = format!("{}:{}", chat_id, day_ymd);
                    let transcript = format_transcript(&full_day);
                    if let Err(e) = ingest_group(&account_id, &key, transcript).await {
                        log::warn!("[imessage] memory write failed key={} err={}", key, e);
                        had_group_failure = true;
                    }
                }

                if had_group_failure {
                    log::warn!(
                        "[imessage] keeping cursor at rowid={} so failed groups retry",
                        last_rowid
                    );
                } else {
                    last_rowid = tick_max_rowid;
                    if let Err(e) = write_cursor(&cursor_path, last_rowid) {
                        log::warn!("[imessage] cursor persist failed err={}", e);
                    }
                }
            }
            Err(e) => {
                log::warn!("[imessage] scan failed err={}", e);
            }
        }

        sleep(SCAN_INTERVAL).await;
    }
}

/// Match a chat identifier against the user-configured allowlist.
///
/// Semantics:
/// - empty list → allow everything (no filter configured)
/// - contains `*` → allow everything
/// - otherwise → exact match on `chat_id` against any entry (whitespace-trimmed)
#[cfg(target_os = "macos")]
fn chat_allowed(chat_id: &str, allowed: &[String]) -> bool {
    if allowed.is_empty() {
        return true;
    }
    let chat_trim = chat_id.trim();
    allowed
        .iter()
        .map(|s| s.trim())
        .any(|entry| entry == "*" || entry.eq_ignore_ascii_case(chat_trim))
}

/// Ask the core for the current iMessage config via JSON-RPC.
///
/// Returns:
/// - `Ok(Some(allowed_contacts))` when iMessage is connected (allow-list may
///   be empty = "all chats")
/// - `Ok(None)` when iMessage is not connected / config absent
/// - `Err(_)` on transport or parse errors (caller should retry next tick)
#[cfg(target_os = "macos")]
async fn fetch_imessage_gate() -> anyhow::Result<Option<Vec<String>>> {
    let url = std::env::var("OPENHUMAN_CORE_RPC_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:7788/rpc".into());
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "openhuman.config_get",
        "params": {}
    });
    let res = http_client().post(&url).json(&body).send().await?;
    if !res.status().is_success() {
        anyhow::bail!("config_get http {}", res.status());
    }
    let v: serde_json::Value = res.json().await?;
    let imessage = v
        .pointer("/result/config/channels_config/imessage")
        .cloned();
    let Some(imessage) = imessage else {
        return Ok(None);
    };
    if imessage.is_null() {
        return Ok(None);
    }
    let contacts = imessage
        .get("allowed_contacts")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(Some(contacts))
}

/// Collect `(chat_identifier, anchor_unix_seconds)` pairs touched by a set
/// of new messages — one entry per unique (chat, local-day).
#[cfg(target_os = "macos")]
fn unique_chat_day_keys(messages: &[chatdb::Message]) -> Vec<(String, i64)> {
    use std::collections::HashMap;
    let mut seen: HashMap<(String, String), i64> = HashMap::new();
    for m in messages {
        let Some(chat) = m.chat_identifier.clone() else {
            continue;
        };
        let secs = apple_ns_to_unix(m.date_ns);
        let ymd = seconds_to_ymd(secs);
        seen.entry((chat, ymd)).or_insert(secs);
    }
    seen.into_iter()
        .map(|((chat, _ymd), anchor_secs)| (chat, anchor_secs))
        .collect()
}

/// Path where the last-seen ROWID cursor is persisted, per account.
#[cfg(target_os = "macos")]
fn cursor_file_path<R: Runtime>(app: &AppHandle<R>, account_id: &str) -> PathBuf {
    let base = tauri::Manager::path(app)
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join(format!("imessage-cursor-{}.txt", account_id))
}

#[cfg(target_os = "macos")]
fn read_cursor(path: &std::path::Path) -> Option<i64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[cfg(target_os = "macos")]
fn write_cursor(path: &std::path::Path, rowid: i64) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, rowid.to_string())
}

#[cfg(target_os = "macos")]
fn chat_db_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join("Library/Messages/chat.db"))
}

/// Apple stores message.date as nanoseconds since 2001-01-01 00:00:00 UTC.
/// Return unix-epoch seconds.
#[cfg(target_os = "macos")]
fn apple_ns_to_unix(ns: i64) -> i64 {
    const APPLE_EPOCH_OFFSET: i64 = 978_307_200;
    ns / 1_000_000_000 + APPLE_EPOCH_OFFSET
}

#[cfg(target_os = "macos")]
fn seconds_to_ymd(secs: i64) -> String {
    // Local timezone — users inspect memory docs by their calendar day, not UTC.
    use chrono::{Local, TimeZone};
    Local
        .timestamp_opt(secs, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "unknown".into())
}

/// Compute the `[start, end)` Apple-epoch-nanosecond half-open interval that
/// covers the local calendar day containing `secs` (unix seconds). Used by
/// `read_chat_day` so we can rebuild the full transcript for a given day.
#[cfg(target_os = "macos")]
fn local_day_bounds_apple_ns(secs: i64) -> (i64, i64) {
    use chrono::{Duration as ChronoDuration, Local, TimeZone};
    const APPLE_EPOCH_OFFSET: i64 = 978_307_200;
    let dt = Local
        .timestamp_opt(secs, 0)
        .single()
        .unwrap_or_else(|| Local.timestamp_opt(0, 0).unwrap());
    let start_of_day = dt
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|n| Local.from_local_datetime(&n).single())
        .map(|d| d.timestamp())
        .unwrap_or(secs);
    let end_of_day = start_of_day + ChronoDuration::days(1).num_seconds();
    let start_apple = (start_of_day - APPLE_EPOCH_OFFSET) * 1_000_000_000;
    let end_apple = (end_of_day - APPLE_EPOCH_OFFSET) * 1_000_000_000;
    (start_apple, end_apple)
}

/// Best-effort extraction of message body from the `attributedBody` blob
/// (NSKeyedArchiver / typedstream format used by newer macOS Messages).
/// We scan for printable UTF-8 runs of at least 2 chars, picking the
/// longest — good enough for plain-text recall even without a full
/// typedstream decoder. Returns None if nothing plausible is found.
#[cfg(target_os = "macos")]
fn extract_text_from_attributed_body(blob: &[u8]) -> Option<String> {
    let mut runs: Vec<Vec<u8>> = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    for &b in blob {
        // ASCII printable + common whitespace only. We deliberately drop
        // high-bit bytes (they're usually typedstream framing — 0x81/0x84
        // etc.) because keeping them produces invalid-UTF-8 runs that get
        // dropped later anyway. Tradeoff: loses emoji / non-Latin glyphs
        // stored in attributedBody. A proper typedstream decoder is a
        // follow-up; for memory recall on plain-text messages this is the
        // 80/20 fix.
        let printable = (0x20..=0x7e).contains(&b) || b == b'\n' || b == b'\t';
        if printable {
            cur.push(b);
        } else if cur.len() >= 2 {
            runs.push(std::mem::take(&mut cur));
        } else {
            cur.clear();
        }
    }
    if cur.len() >= 2 {
        runs.push(cur);
    }
    // Pick the longest run that decodes as valid UTF-8 and isn't an
    // obvious typedstream type marker (e.g. "NSString", "NSMutableString",
    // "NSDictionary", "iI"/"NSObject" header bytes).
    let ignored_markers = [
        "NSString",
        "NSMutableString",
        "NSAttributedString",
        "NSMutableAttributedString",
        "NSDictionary",
        "NSMutableDictionary",
        "NSObject",
        "NSArray",
        "NSMutableArray",
        "NSNumber",
        "NSData",
        "streamtyped",
    ];
    runs.into_iter()
        .filter_map(|r| String::from_utf8(r).ok())
        .filter(|s| {
            let trimmed = s.trim();
            trimmed.len() >= 2 && !ignored_markers.iter().any(|m| trimmed == *m)
        })
        .max_by_key(|s| s.len())
        .map(|s| s.trim().to_string())
}

#[cfg(target_os = "macos")]
fn format_transcript(messages: &[chatdb::Message]) -> String {
    let mut out = String::new();
    for m in messages {
        let sender = if m.is_from_me {
            "me".to_string()
        } else {
            m.handle_id.clone().unwrap_or_else(|| "unknown".into())
        };
        let body = message_body(m);
        let text = body.replace('\n', " ");
        if text.is_empty() {
            // Pure attachment / reaction with no recoverable text — keep
            // the envelope so the timeline stays complete but mark it.
            let ts = apple_ns_to_unix(m.date_ns);
            out.push_str(&format!("[{}] {}: [non-text]\n", ts, sender));
            continue;
        }
        let ts = apple_ns_to_unix(m.date_ns);
        out.push_str(&format!("[{}] {}: {}\n", ts, sender, text));
    }
    out
}

/// Return the best available body for a message: prefer `text`, then fall
/// back to a heuristic string extracted from `attributedBody` (the binary
/// body that newer macOS versions use when `text` is NULL).
#[cfg(target_os = "macos")]
fn message_body(m: &chatdb::Message) -> String {
    if let Some(t) = m.text.as_deref() {
        if !t.is_empty() {
            return t.to_string();
        }
    }
    if let Some(blob) = m.attributed_body.as_deref() {
        if let Some(decoded) = extract_text_from_attributed_body(blob) {
            return decoded;
        }
    }
    String::new()
}

#[cfg(target_os = "macos")]
async fn ingest_group(account_id: &str, key: &str, transcript: String) -> anyhow::Result<()> {
    let (chat_id, day) = key.split_once(':').unwrap_or((key, ""));
    let url = std::env::var("OPENHUMAN_CORE_RPC_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:7788/rpc".into());

    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "openhuman.memory_doc_ingest",
        "params": {
            "namespace": format!("imessage:{}", account_id),
            "key": key,
            "title": format!("Messages — {} — {}", chat_id, day),
            "content": transcript,
            "source_type": "imessage",
            "tags": ["chat", "imessage"],
            "metadata": {
                "chat_identifier": chat_id,
                "day": day,
                "source": "imessage"
            },
            "category": "chat"
        }
    });

    let res = http_client().post(&url).json(&body).send().await?;

    if !res.status().is_success() {
        anyhow::bail!("core rpc {}: {}", res.status(), res.text().await?);
    }

    log::info!("[imessage] memory upsert ok key={}", key);
    Ok(())
}

// Non-macOS stub so the rest of the app compiles unchanged.
#[cfg(not(target_os = "macos"))]
pub struct ScannerRegistry;

#[cfg(not(target_os = "macos"))]
impl ScannerRegistry {
    pub fn new() -> Self {
        Self
    }
    pub fn ensure_scanner<R: tauri::Runtime>(
        self: std::sync::Arc<Self>,
        _app: tauri::AppHandle<R>,
        _account_id: String,
    ) {
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn apple_ns_to_unix_converts_apple_epoch_zero() {
        assert_eq!(apple_ns_to_unix(0), 978_307_200);
    }

    #[test]
    fn apple_ns_to_unix_converts_one_second_past_apple_epoch() {
        assert_eq!(apple_ns_to_unix(1_000_000_000), 978_307_201);
    }

    #[test]
    fn seconds_to_ymd_formats_known_date_in_local_tz() {
        // 2001-01-01 00:00:00 UTC. In US timezones this falls on 2000-12-31
        // in local time, so assert only the shape (YYYY-MM-DD) and that the
        // year is 2000 or 2001 — keeps the test robust across CI timezones.
        let out = seconds_to_ymd(978_307_200);
        assert_eq!(out.len(), 10);
        assert!(
            out.starts_with("2000-") || out.starts_with("2001-"),
            "got {}",
            out
        );
    }

    #[test]
    fn extract_text_from_attributed_body_finds_message() {
        // Fake typedstream-style blob with 'hello world' as the longest
        // printable run embedded between type markers.
        let mut blob = b"streamtyped\x81\xe8\x03\x84\x01@\x84\x84\x84\x08NSString\x00\x84\x84\x08NSObject\x00\x85\x84\x01+\x0bhello world\x86".to_vec();
        blob.extend_from_slice(b"\x00\x00\x00");
        let out = extract_text_from_attributed_body(&blob).unwrap_or_default();
        assert!(out.contains("hello world"), "got {:?}", out);
    }

    #[test]
    fn message_body_prefers_text_then_attributed_body() {
        let m = chatdb::Message {
            rowid: 1,
            guid: None,
            text: Some("direct".into()),
            attributed_body: Some(b"ignored".to_vec()),
            date_ns: 0,
            is_from_me: false,
            handle_id: None,
            chat_identifier: None,
            chat_name: None,
            service: None,
        };
        assert_eq!(message_body(&m), "direct");

        let m2 = chatdb::Message {
            rowid: 2,
            guid: None,
            text: None,
            attributed_body: Some(b"\x00\x00fallback body\x00".to_vec()),
            date_ns: 0,
            is_from_me: false,
            handle_id: None,
            chat_identifier: None,
            chat_name: None,
            service: None,
        };
        let body = message_body(&m2);
        assert!(body.contains("fallback body"), "got {:?}", body);
    }

    #[test]
    fn chat_allowed_empty_list_allows_all() {
        assert!(chat_allowed("+15551234567", &[]));
    }

    #[test]
    fn chat_allowed_wildcard_allows_all() {
        assert!(chat_allowed("+15551234567", &["*".to_string()]));
    }

    #[test]
    fn chat_allowed_matches_exact_entry_case_insensitive() {
        let allowed = vec!["+15551234567".to_string(), "USER@Example.com".to_string()];
        assert!(chat_allowed("+15551234567", &allowed));
        assert!(chat_allowed("user@example.com", &allowed));
        assert!(!chat_allowed("+15550000000", &allowed));
    }

    #[test]
    fn format_transcript_renders_known_messages() {
        let msgs = vec![
            chatdb::Message {
                rowid: 1,
                guid: None,
                text: Some("hi".into()),
                attributed_body: None,
                date_ns: 0,
                is_from_me: false,
                handle_id: Some("+15551234567".into()),
                chat_identifier: Some("+15551234567".into()),
                chat_name: None,
                service: None,
            },
            chatdb::Message {
                rowid: 2,
                guid: None,
                text: Some("yo".into()),
                attributed_body: None,
                date_ns: 0,
                is_from_me: true,
                handle_id: None,
                chat_identifier: Some("+15551234567".into()),
                chat_name: None,
                service: None,
            },
        ];
        let transcript = format_transcript(&msgs);
        let groups =
            std::collections::HashMap::from([("+15551234567:day".to_string(), transcript.clone())]);
        let _ = groups;
        assert_eq!(groups.len(), 1);
        let transcript = groups.values().next().expect("one group").clone();
        assert!(transcript.contains("hi"));
        assert!(transcript.contains("yo"));
        assert!(transcript.contains("me:"));
    }

    /// Real chat.db integration test. Gated with `#[ignore]` — run with
    /// `cargo test --manifest-path app/src-tauri/Cargo.toml \
    ///   imessage_scanner -- --ignored`. Requires Full Disk Access granted
    /// to the test-runner binary. Asserts we can open chat.db read-only,
    /// run our JOIN query, and deserialize at least one row.
    #[test]
    #[ignore]
    fn real_chat_db_opens_and_returns_messages() {
        let path = match chat_db_path() {
            Some(p) => p,
            None => {
                eprintln!("HOME not set — skipping");
                return;
            }
        };
        if !path.exists() {
            eprintln!("chat.db not found at {} — skipping", path.display());
            return;
        }
        let msgs = match chatdb::read_since(&path, 0, 5) {
            Ok(m) => m,
            Err(e) => panic!("read_since failed: {}", e),
        };
        assert!(
            !msgs.is_empty(),
            "expected at least one message from a real chat.db — is it empty?"
        );
        // Each message should have a rowid and a date_ns in Apple-epoch range.
        for m in &msgs {
            assert!(m.rowid > 0);
            assert!(m.date_ns >= 0);
        }
    }

    /// Sanity: `read_since` with cursor past max rowid returns empty.
    #[test]
    #[ignore]
    fn real_chat_db_empty_past_cursor() {
        let path = match chat_db_path() {
            Some(p) => p,
            None => return,
        };
        if !path.exists() {
            return;
        }
        // rowid way past any real value
        let msgs = chatdb::read_since(&path, i64::MAX - 1, 10).unwrap();
        assert!(msgs.is_empty());
    }
}
