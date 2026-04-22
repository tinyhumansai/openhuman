//! High-level operations for the Gmail domain (connect, disconnect, sync, list).
//!
//! All handlers delegate storage to `store.rs` and ingestion to `ingest.rs`.
//! They are re-exported from `mod.rs` as `rpc` for the controller schema
//! layer to call.

use chrono::Utc;

use crate::openhuman::config::Config;
use crate::openhuman::gmail::ingest::ingest_batch;
use crate::openhuman::gmail::store;
use crate::openhuman::gmail::types::{GmailAccount, GmailSyncStats};
use crate::rpc::RpcOutcome;

// ---------------------------------------------------------------------------
// list_accounts
// ---------------------------------------------------------------------------

/// Return all connected Gmail accounts with their last-sync stats.
pub async fn list_accounts(config: &Config) -> Result<RpcOutcome<Vec<GmailSyncStats>>, String> {
    log::debug!("[gmail][ops] list_accounts");
    let accounts = store::list_accounts(config).map_err(|e| {
        log::warn!("[gmail][ops] list_accounts store error: {}", e);
        e.to_string()
    })?;
    let stats: Vec<GmailSyncStats> = accounts.iter().map(GmailSyncStats::from).collect();
    log::info!("[gmail][ops] list_accounts -> {} accounts", stats.len());
    Ok(RpcOutcome::new(stats, vec![]))
}

// ---------------------------------------------------------------------------
// connect_account
// ---------------------------------------------------------------------------

/// Register a Gmail account in the domain store and optionally register a
/// 15-minute cron job for periodic re-sync.
///
/// `account_id` is the caller-supplied opaque id (matching the webview
/// `account_id`). `email` is the Google account address.
///
/// Returns the upserted `GmailSyncStats`.
pub async fn connect_account(
    config: &Config,
    account_id: &str,
    email: &str,
) -> Result<RpcOutcome<GmailSyncStats>, String> {
    log::info!(
        "[gmail][ops] connect_account account_id={} email={}",
        account_id,
        email
    );

    let now_ms = Utc::now().timestamp_millis();

    // Register a recurring 15-minute cron job that calls `gmail.sync_now`
    // on behalf of this account via the controller registry.
    let cron_command = format!("openhuman.gmail_sync_now account_id={}", account_id);
    let schedule = crate::openhuman::cron::Schedule::Every {
        every_ms: 15 * 60 * 1000,
    };
    let job_name = format!("gmail-sync-{}", account_id);
    let cron_job_id = match crate::openhuman::cron::add_shell_job(
        config,
        Some(job_name),
        schedule,
        &cron_command,
    ) {
        Ok(job) => {
            log::info!(
                "[gmail][ops] registered cron job_id={} account_id={}",
                job.id,
                account_id
            );
            Some(job.id)
        }
        Err(e) => {
            log::warn!(
                "[gmail][ops] could not register cron job for account_id={}: {} (continuing)",
                account_id,
                e
            );
            None
        }
    };

    let account = GmailAccount {
        account_id: account_id.to_string(),
        email: email.to_string(),
        connected_at_ms: now_ms,
        last_sync_at_ms: 0,
        last_sync_count: 0,
        cron_job_id,
    };

    store::upsert_account(config, &account).map_err(|e| {
        log::warn!(
            "[gmail][ops] upsert_account failed account_id={}: {}",
            account_id,
            e
        );
        e.to_string()
    })?;

    log::info!(
        "[gmail][ops] connect_account done account_id={} email={}",
        account_id,
        email
    );
    Ok(RpcOutcome::new(GmailSyncStats::from(&account), vec![]))
}

// ---------------------------------------------------------------------------
// disconnect_account
// ---------------------------------------------------------------------------

/// Disconnect a Gmail account: cancel its cron job, wipe memory namespace,
/// and remove the store row.
pub async fn disconnect_account(
    config: &Config,
    account_id: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    log::info!("[gmail][ops] disconnect_account account_id={}", account_id);

    // Retrieve current account so we know the email (for namespace wipe).
    let account_opt = store::get_account(config, account_id).map_err(|e| e.to_string())?;

    // Cancel cron job if present.
    if let Some(ref account) = account_opt {
        if let Some(ref job_id) = account.cron_job_id {
            log::debug!(
                "[gmail][ops] removing cron job_id={} for account_id={}",
                job_id,
                account_id
            );
            if let Err(e) = crate::openhuman::cron::remove_job(config, job_id) {
                log::warn!(
                    "[gmail][ops] remove cron job_id={} failed (non-fatal): {}",
                    job_id,
                    e
                );
            }
        }

        // Wipe memory namespace.
        let namespace = format!("skill:gmail:{}", account.email);
        log::info!(
            "[gmail][ops] deleting memory namespace={} for account_id={}",
            namespace,
            account_id
        );
        match crate::openhuman::memory::global::client() {
            Ok(memory) => {
                if let Err(e) = memory.clear_namespace(&namespace).await {
                    log::warn!(
                        "[gmail][ops] delete namespace={} failed (non-fatal): {}",
                        namespace,
                        e
                    );
                }
            }
            Err(e) => {
                log::warn!(
                    "[gmail][ops] memory client unavailable for namespace wipe: {}",
                    e
                );
            }
        }
    }

    // Remove store row.
    store::remove_account(config, account_id).map_err(|e| e.to_string())?;

    log::info!(
        "[gmail][ops] disconnect_account done account_id={}",
        account_id
    );
    Ok(RpcOutcome::new(
        serde_json::json!({
            "account_id": account_id,
            "disconnected": true,
        }),
        vec![],
    ))
}

// ---------------------------------------------------------------------------
// sync_now
// ---------------------------------------------------------------------------

/// Trigger an on-demand sync for one account. In the Rust domain this is
/// lightweight — it publishes a `GmailMessagesIngested` event with count=0
/// so the UI can poll for actual messages. The heavy lifting (IDB / network
/// MITM) happens in the Tauri-side `gmail_scanner` and forwards messages
/// here via the `gmail_ingest_messages` Tauri command.
pub async fn sync_now(
    config: &Config,
    account_id: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    log::info!("[gmail][ops] sync_now account_id={}", account_id);

    let account = store::get_account(config, account_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("gmail account not found: {}", account_id))?;

    // Publish a domain event so the bus subscriber can trigger scanner
    // actions or downstream logic.
    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::GmailMessagesIngested {
            account_id: account_id.to_string(),
            count: 0,
        },
    );

    log::debug!(
        "[gmail][ops] sync_now published event account_id={}",
        account_id
    );
    Ok(RpcOutcome::new(
        serde_json::json!({
            "account_id": account_id,
            "email": account.email,
            "status": "sync_triggered",
        }),
        vec![],
    ))
}

// ---------------------------------------------------------------------------
// get_stats
// ---------------------------------------------------------------------------

/// Return sync stats for a single account.
pub async fn get_stats(
    config: &Config,
    account_id: &str,
) -> Result<RpcOutcome<GmailSyncStats>, String> {
    log::debug!("[gmail][ops] get_stats account_id={}", account_id);
    let account = store::get_account(config, account_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("gmail account not found: {}", account_id))?;
    Ok(RpcOutcome::new(GmailSyncStats::from(&account), vec![]))
}

// ---------------------------------------------------------------------------
// ingest_raw_response (called via core RPC from GmailPanel JS forwarder)
// ---------------------------------------------------------------------------

/// Ingest a raw Gmail sync response body captured by the CDP MITM scanner.
///
/// The body may have Gmail's JSONP prefix (`)]}'\n`) which is stripped before
/// JSON parsing. The parser then walks the nested array structure defensively
/// to extract `GmailMessage`-shaped records and calls `ingest_batch`.
///
/// Parsing failures are logged at debug level and do not propagate — we never
/// crash on a malformed response body.
pub async fn ingest_raw_response(
    config: &Config,
    account_id: &str,
    url: &str,
    body: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    log::debug!(
        "[gmail][ops] ingest_raw_response account_id={} url_len={} body_bytes={}",
        account_id,
        url.len(),
        body.len()
    );

    let account = store::get_account(config, account_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("gmail account not found: {}", account_id))?;

    // Strip Gmail's JSONP prefix if present. Gmail's batch-RPC endpoints
    // return `)]}'` followed by a newline before the JSON payload to prevent
    // cross-site script inclusion. We strip the prefix so we can parse the
    // remainder as normal JSON.
    let json_body = strip_jsonp_prefix(body);

    let messages = parse_sync_response(json_body, account_id);
    log::info!(
        "[gmail][ops] ingest_raw_response extracted {} messages account_id={}",
        messages.len(),
        account_id
    );

    if messages.is_empty() {
        return Ok(RpcOutcome::new(
            serde_json::json!({
                "account_id": account_id,
                "ingested": 0,
                "errors": 0,
            }),
            vec![],
        ));
    }

    let memory = crate::openhuman::memory::global::client()?;
    let (ok, err) = ingest_batch(&memory, &account.email, &messages).await;

    if let Err(e) = store::update_sync_cursor(config, account_id, ok as i64) {
        log::warn!(
            "[gmail][ops] update_sync_cursor failed account_id={}: {}",
            account_id,
            e
        );
    }

    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::GmailMessagesIngested {
            account_id: account_id.to_string(),
            count: ok,
        },
    );

    log::info!(
        "[gmail][ops] ingest_raw_response done account_id={} ingested={} errors={}",
        account_id,
        ok,
        err
    );
    Ok(RpcOutcome::new(
        serde_json::json!({
            "account_id": account_id,
            "ingested": ok,
            "errors": err,
        }),
        vec![],
    ))
}

// ---------------------------------------------------------------------------
// JSONP stripping + Gmail sync response parsing
// ---------------------------------------------------------------------------

/// Strip Gmail's `)]}'` JSONP-guard prefix (optionally followed by `\n`).
/// Returns the remaining string which should be valid JSON.
fn strip_jsonp_prefix(body: &str) -> &str {
    // Gmail's JSONP prefix variants observed in the wild:
    //   `)]}'` (4 chars, no newline)
    //   `)]}\'\n` (5 chars with trailing newline — most common)
    let prefixes: &[&str] = &[")]}'\\n", ")]}'\n", ")]}'"];
    for prefix in prefixes {
        if body.starts_with(prefix) {
            return &body[prefix.len()..];
        }
    }
    body
}

/// Walk a parsed Gmail sync response and extract `GmailMessage` records.
///
/// Gmail's jslayout encoding is a deeply nested array. We perform a defensive
/// recursive walk looking for sub-arrays that appear to be message envelopes:
/// the first element looks like a 16-char hex message id and we find
/// header-like sub-arrays nearby. Parsing failures at any node are silently
/// skipped — we collect whatever we can.
fn parse_sync_response(
    json_str: &str,
    account_id: &str,
) -> Vec<crate::openhuman::gmail::types::GmailMessage> {
    use serde_json::Value;

    let v: Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(e) => {
            log::debug!(
                "[gmail][ops] parse_sync_response parse error account_id={}: {}",
                account_id,
                e
            );
            return vec![];
        }
    };

    let mut messages = Vec::new();
    extract_messages_from_value(&v, &mut messages, 0);
    log::debug!(
        "[gmail][ops] parse_sync_response extracted={} account_id={}",
        messages.len(),
        account_id
    );
    messages
}

const MAX_RECURSION_DEPTH: usize = 15;

fn extract_messages_from_value(
    v: &serde_json::Value,
    out: &mut Vec<crate::openhuman::gmail::types::GmailMessage>,
    depth: usize,
) {
    use serde_json::Value;
    if depth > MAX_RECURSION_DEPTH {
        return;
    }
    match v {
        Value::Array(arr) => {
            // Heuristic: a Gmail message envelope starts with a string that
            // looks like a message id (hex, 16 chars), and a thread id nearby.
            // We look for this pattern and try to extract fields.
            if let Some(msg) = try_extract_message(arr) {
                out.push(msg);
                return; // don't recurse into a message we already parsed
            }
            for item in arr {
                extract_messages_from_value(item, out, depth + 1);
            }
        }
        Value::Object(map) => {
            for val in map.values() {
                extract_messages_from_value(val, out, depth + 1);
            }
        }
        _ => {}
    }
}

/// Try to interpret `arr` as a Gmail message envelope.
/// Returns `Some(GmailMessage)` if the array matches the expected shape,
/// `None` if it doesn't look like a message.
fn try_extract_message(
    arr: &[serde_json::Value],
) -> Option<crate::openhuman::gmail::types::GmailMessage> {
    use serde_json::Value;

    // Gmail message arrays are long (30+ elements). A very short array is
    // definitely not a message.
    if arr.len() < 5 {
        return None;
    }

    // Element 0: message id (hex string, ~16 chars)
    let id = arr.first()?.as_str()?;
    if id.len() < 8 || id.len() > 32 {
        return None;
    }
    if !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }

    // Element 1: thread id (also hex string)
    let thread_id = arr
        .get(1)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Walk remaining elements looking for header-like sub-arrays and labels.
    let mut from = String::new();
    let mut to = String::new();
    let mut subject = String::new();
    let mut snippet = String::new();
    let mut body = String::new();
    let mut labels: Vec<String> = Vec::new();
    let mut ts_ms: i64 = 0;

    for element in arr.iter().skip(2) {
        match element {
            Value::Array(sub) => {
                // Headers are typically arrays of [name, value] pairs.
                extract_headers_from_subarray(sub, &mut from, &mut to, &mut subject);
                // Labels are arrays of strings like "INBOX", "UNREAD", ...
                extract_labels_from_subarray(sub, &mut labels);
                // Snippet / body text
                extract_body_from_subarray(sub, &mut snippet, &mut body);
            }
            Value::Number(n) => {
                // Timestamps appear as large integers (ms since epoch).
                if ts_ms == 0 {
                    let candidate = n.as_i64().unwrap_or(0);
                    // Sanity check: must be a plausible epoch ms after year 2000.
                    if candidate > 946_684_800_000 {
                        ts_ms = candidate;
                    }
                }
            }
            _ => {}
        }
    }

    // Require at minimum an id and at least one non-empty useful field.
    if from.is_empty() && subject.is_empty() && snippet.is_empty() {
        return None;
    }

    Some(crate::openhuman::gmail::types::GmailMessage {
        id: id.to_string(),
        thread_id,
        from,
        to,
        subject,
        snippet,
        body,
        labels,
        ts_ms,
    })
}

/// Walk a sub-array looking for `[header_name, header_value]` pairs.
fn extract_headers_from_subarray(
    arr: &[serde_json::Value],
    from: &mut String,
    to: &mut String,
    subject: &mut String,
) {
    use serde_json::Value;
    // Pattern: ["From", "Alice <alice@example.com>"]
    if arr.len() == 2 {
        if let (Some(name), Some(val)) = (arr[0].as_str(), arr[1].as_str()) {
            match name.to_lowercase().as_str() {
                "from" => {
                    if from.is_empty() {
                        *from = val.to_string();
                    }
                }
                "to" => {
                    if to.is_empty() {
                        *to = val.to_string();
                    }
                }
                "subject" => {
                    if subject.is_empty() {
                        *subject = val.to_string();
                    }
                }
                _ => {}
            }
        }
    }
    // Recurse one level for nested header arrays.
    for item in arr {
        if let Value::Array(sub) = item {
            if sub.len() == 2 {
                extract_headers_from_subarray(sub, from, to, subject);
            }
        }
    }
}

/// Walk a sub-array looking for Gmail system labels (all-caps ASCII strings).
fn extract_labels_from_subarray(arr: &[serde_json::Value], labels: &mut Vec<String>) {
    use serde_json::Value;
    for item in arr {
        match item {
            Value::String(s) => {
                let upper = s.to_uppercase();
                // Gmail system labels are typically all-caps and match known names.
                if matches!(
                    upper.as_str(),
                    "INBOX"
                        | "SENT"
                        | "DRAFT"
                        | "SPAM"
                        | "TRASH"
                        | "UNREAD"
                        | "STARRED"
                        | "IMPORTANT"
                        | "CATEGORY_PERSONAL"
                        | "CATEGORY_SOCIAL"
                        | "CATEGORY_PROMOTIONS"
                        | "CATEGORY_UPDATES"
                        | "CATEGORY_FORUMS"
                ) && !labels.contains(&upper)
                {
                    labels.push(upper);
                }
            }
            Value::Array(sub) => {
                extract_labels_from_subarray(sub, labels);
            }
            _ => {}
        }
    }
}

/// Walk a sub-array looking for a non-empty body / snippet string.
/// Heuristic: a string longer than 20 chars that doesn't look like a
/// header name or label is likely the body or snippet.
fn extract_body_from_subarray(arr: &[serde_json::Value], snippet: &mut String, body: &mut String) {
    use serde_json::Value;
    for item in arr {
        if let Value::String(s) = item {
            let trimmed = s.trim();
            if trimmed.len() > 20 && !trimmed.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                if snippet.is_empty() {
                    *snippet = trimmed.chars().take(200).collect::<String>();
                } else if body.is_empty() && trimmed.len() > snippet.len() {
                    *body = trimmed.to_string();
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ingest_messages (called by Tauri-side scanner)
// ---------------------------------------------------------------------------

/// Ingest a batch of messages into memory on behalf of `account_id`.
/// Updates the sync cursor after ingestion.
pub async fn ingest_messages(
    config: &Config,
    account_id: &str,
    messages: Vec<crate::openhuman::gmail::types::GmailMessage>,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    log::info!(
        "[gmail][ops] ingest_messages account_id={} count={}",
        account_id,
        messages.len()
    );

    let account = store::get_account(config, account_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("gmail account not found: {}", account_id))?;

    let memory = crate::openhuman::memory::global::client()?;
    let (ok, err) = ingest_batch(&memory, &account.email, &messages).await;

    // Update sync cursor.
    if let Err(e) = store::update_sync_cursor(config, account_id, ok as i64) {
        log::warn!(
            "[gmail][ops] update_sync_cursor failed account_id={}: {}",
            account_id,
            e
        );
    }

    // Publish domain event.
    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::GmailMessagesIngested {
            account_id: account_id.to_string(),
            count: ok,
        },
    );

    log::info!(
        "[gmail][ops] ingest_messages done account_id={} ok={} err={}",
        account_id,
        ok,
        err
    );
    Ok(RpcOutcome::new(
        serde_json::json!({
            "account_id": account_id,
            "ingested": ok,
            "errors": err,
        }),
        vec![],
    ))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // The fixture contains the JSONP prefix `)]}'` followed by a newline and
    // a JSON array with 3 synthetic message envelopes.
    const FIXTURE: &str = include_str!("fixtures/sync_response.json");

    #[test]
    fn strip_jsonp_prefix_removes_prefix() {
        let input = ")]}'\nsome json";
        assert_eq!(strip_jsonp_prefix(input), "some json");
    }

    #[test]
    fn strip_jsonp_prefix_noop_on_plain_json() {
        let input = r#"{"key": "value"}"#;
        assert_eq!(strip_jsonp_prefix(input), input);
    }

    #[test]
    fn parse_sync_response_extracts_messages_from_fixture() {
        let json_body = strip_jsonp_prefix(FIXTURE);
        let messages = parse_sync_response(json_body, "test");
        // The fixture has 3 synthetic messages; we should extract at least 2.
        assert!(
            messages.len() >= 2,
            "expected >= 2 messages, got {}",
            messages.len()
        );
    }

    #[test]
    fn parse_sync_response_extracts_correct_fields() {
        let json_body = strip_jsonp_prefix(FIXTURE);
        let messages = parse_sync_response(json_body, "test");
        // First message should have Alice as sender.
        let first = messages
            .iter()
            .find(|m| m.from.contains("alice@example.com"));
        assert!(first.is_some(), "expected a message from alice@example.com");
        let first = first.unwrap();
        assert!(
            first.subject.contains("Meeting notes"),
            "unexpected subject: {}",
            first.subject
        );
    }

    #[test]
    fn parse_sync_response_detects_unread_label() {
        let json_body = strip_jsonp_prefix(FIXTURE);
        let messages = parse_sync_response(json_body, "test");
        let unread = messages.iter().find(|m| m.is_unread());
        assert!(
            unread.is_some(),
            "expected at least one UNREAD message in fixture"
        );
    }

    #[test]
    fn parse_sync_response_noop_on_garbage() {
        let messages = parse_sync_response("not json at all !!!", "test");
        assert!(messages.is_empty());
    }

    #[test]
    fn parse_sync_response_noop_on_empty_body() {
        let messages = parse_sync_response("", "test");
        assert!(messages.is_empty());
    }

    #[test]
    fn parse_sync_response_noop_on_empty_json_array() {
        let messages = parse_sync_response("[]", "test");
        assert!(messages.is_empty());
    }
}
