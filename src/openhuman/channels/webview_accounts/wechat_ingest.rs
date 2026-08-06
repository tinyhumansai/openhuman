//! WeChat Web ingest contract — normalized payloads for context + memory.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WechatChatRow {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(default)]
    pub unread: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WechatMessageRow {
    pub chat_id: String,
    pub chat_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WechatScanPayload {
    pub account_id: String,
    #[serde(default)]
    pub chat_rows: Vec<WechatChatRow>,
    #[serde(default)]
    pub messages: Vec<WechatMessageRow>,
    #[serde(default)]
    pub unread: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub snapshot_key: String,
    #[serde(default = "default_source")]
    pub source: String,
}

fn default_source() -> String {
    "cdp-dom".to_string()
}

pub fn validate_scan(payload: &WechatScanPayload) -> Result<(), String> {
    if payload.account_id.trim().is_empty() {
        return Err("account_id is required".into());
    }
    if payload.chat_rows.is_empty() && payload.messages.is_empty() {
        return Err("scan has no chat rows or messages".into());
    }
    Ok(())
}

pub fn list_ingest_envelope(
    account_id: &str,
    payload: &WechatScanPayload,
    ts_millis: i64,
) -> Value {
    json!({
        "account_id": account_id,
        "provider": "wechat",
        "kind": "ingest",
        "payload": list_ingest_payload(payload),
        "ts": ts_millis,
    })
}

pub fn list_ingest_payload(payload: &WechatScanPayload) -> Value {
    let messages: Vec<Value> = payload
        .chat_rows
        .iter()
        .enumerate()
        .map(|(idx, row)| {
            let id = if row.name.is_empty() {
                format!("wechat:row:{idx}")
            } else {
                format!("wechat:{idx}:{}", row.name)
            };
            json!({
                "id": id,
                "from": if row.name.is_empty() { Value::Null } else { json!(row.name) },
                "body": row.preview.clone().map(Value::String).unwrap_or(Value::Null),
                "unread": row.unread,
            })
        })
        .collect();
    json!({
        "messages": messages,
        "unread": payload.unread,
        "snapshotKey": payload.snapshot_key,
    })
}

pub fn memory_doc_ingest_list_snapshot(
    payload: &WechatScanPayload,
) -> Result<Map<String, Value>, String> {
    validate_scan(payload)?;
    if payload.chat_rows.is_empty() {
        return Err("no chat rows for list snapshot".into());
    }
    let namespace = format!("wechat-web:{}", payload.account_id);
    let key = if payload.snapshot_key.is_empty() {
        format!("list:{}", chrono_day_key())
    } else {
        format!("list:{}", payload.snapshot_key)
    };
    Ok(memory_doc_params(
        namespace,
        key,
        format!(
            "WeChat · chat list · {}",
            short_account(&payload.account_id)
        ),
        format_list_transcript(payload),
        json!({
            "provider": "wechat",
            "account_id": payload.account_id,
            "kind": "chat-list",
            "chat_count": payload.chat_rows.len(),
            "unread": payload.unread,
        }),
        vec!["wechat", "chat-list"],
    ))
}

pub fn memory_doc_ingest_peer_transcript(
    account_id: &str,
    chat_id: &str,
    chat_name: &str,
    rows: &[WechatMessageRow],
) -> Result<Map<String, Value>, String> {
    if account_id.trim().is_empty() {
        return Err("account_id is required".into());
    }
    if chat_id.trim().is_empty() {
        return Err("chat_id is required".into());
    }
    if rows.is_empty() {
        return Err("no messages for peer transcript".into());
    }
    let mut sorted: Vec<&WechatMessageRow> = rows.iter().collect();
    sorted.sort_by_key(|m| m.ts.unwrap_or(0));
    let first_day = ts_to_ymd(sorted.first().and_then(|m| m.ts).unwrap_or(0));
    let last_day = ts_to_ymd(sorted.last().and_then(|m| m.ts).unwrap_or(0));
    let transcript: String = sorted
        .iter()
        .map(|m| {
            let stamp = m.ts.map(format_message_stamp).unwrap_or_else(|| "?".into());
            let who = m.sender.as_deref().filter(|s| !s.is_empty()).unwrap_or("?");
            format!("[{stamp}] {who}: {}", m.body.replace(['\r', '\n'], " "))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let peer_label = if chat_name.trim().is_empty() {
        chat_id
    } else {
        chat_name
    };
    let header = format!(
        "# WeChat — {peer_label}\nchat_id: {chat_id}\naccount_id: {account_id}\nmessages: {}\nrange: {first_day} → {last_day}\n\n",
        sorted.len()
    );
    let key = if peer_key_looks_clean(chat_name) {
        format!("{chat_id}:{chat_name}")
    } else {
        chat_id.to_string()
    };
    Ok(memory_doc_params(
        format!("wechat-web:{account_id}"),
        key,
        format!("WeChat · {peer_label}"),
        format!("{header}{transcript}"),
        json!({
            "provider": "wechat",
            "account_id": account_id,
            "chat_id": chat_id,
            "chat_name": chat_name,
            "message_count": sorted.len(),
        }),
        vec!["wechat", "peer-transcript"],
    ))
}

fn memory_doc_params(
    namespace: String,
    key: String,
    title: String,
    content: String,
    metadata: Value,
    tags: Vec<&str>,
) -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("namespace".into(), json!(namespace));
    params.insert("key".into(), json!(key));
    params.insert("title".into(), json!(title));
    params.insert("content".into(), json!(content));
    params.insert("source_type".into(), json!("wechat-web"));
    params.insert("priority".into(), json!("medium"));
    params.insert("tags".into(), json!(tags));
    params.insert("metadata".into(), metadata);
    params.insert("category".into(), json!("core"));
    params
}

fn format_list_transcript(payload: &WechatScanPayload) -> String {
    let mut lines = vec![
        "# WeChat — chat list".to_string(),
        format!("account_id: {}", payload.account_id),
        format!("chats: {}", payload.chat_rows.len()),
        format!("unread: {}", payload.unread),
        String::new(),
    ];
    for row in &payload.chat_rows {
        let preview = row.preview.as_deref().unwrap_or("");
        let badge = if row.unread > 0 {
            format!(" [{} unread]", row.unread)
        } else {
            String::new()
        };
        lines.push(format!("- {}{}: {}", row.name, badge, preview));
    }
    lines.join("\n")
}

fn short_account(account_id: &str) -> String {
    if account_id.chars().count() <= 8 {
        account_id.to_string()
    } else {
        account_id.chars().take(8).collect()
    }
}

fn peer_key_looks_clean(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

fn format_message_stamp(ts: i64) -> String {
    let day = ts_to_ymd(ts);
    let secs_of_day = (ts.rem_euclid(86_400)) as u32;
    format!(
        "{} {:02}:{:02}Z",
        day,
        secs_of_day / 3600,
        (secs_of_day / 60) % 60
    )
}

fn ts_to_ymd(secs: i64) -> String {
    if secs <= 0 {
        return String::new();
    }
    let days = secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y_real = (if m <= 2 { y + 1 } else { y }) as i32;
    format!("{:04}-{:02}-{:02}", y_real, m, d)
}

fn chrono_day_key() -> String {
    ts_to_ymd(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_empty_account() {
        let mut p = WechatScanPayload {
            account_id: "acct".into(),
            chat_rows: vec![WechatChatRow {
                name: "A".into(),
                preview: None,
                unread: 0,
            }],
            messages: vec![],
            unread: 0,
            snapshot_key: String::new(),
            source: "cdp-dom".into(),
        };
        p.account_id = "  ".into();
        assert!(validate_scan(&p).is_err());
    }

    #[test]
    fn validate_rejects_empty_scan() {
        assert!(validate_scan(&WechatScanPayload {
            account_id: "acct".into(),
            chat_rows: vec![],
            messages: vec![],
            unread: 0,
            snapshot_key: String::new(),
            source: "cdp-dom".into(),
        })
        .is_err());
    }

    #[test]
    fn list_ingest_payload_has_messages() {
        let v = list_ingest_payload(&WechatScanPayload {
            account_id: "a".into(),
            chat_rows: vec![WechatChatRow {
                name: "Bob".into(),
                preview: Some("hi".into()),
                unread: 1,
            }],
            messages: vec![],
            unread: 1,
            snapshot_key: "k".into(),
            source: "cdp-dom".into(),
        });
        assert_eq!(v["messages"].as_array().map(|a| a.len()), Some(1));
    }

    #[test]
    fn peer_transcript_rejects_empty_messages() {
        assert!(memory_doc_ingest_peer_transcript("acct", "c1", "Alice", &[]).is_err());
    }

    #[test]
    fn peer_transcript_key_includes_chat_id_for_clean_names() {
        let rows = vec![WechatMessageRow {
            chat_id: "chat-1".into(),
            chat_name: "Alice".into(),
            sender: None,
            body: "hello".into(),
            ts: Some(1),
        }];

        let first = memory_doc_ingest_peer_transcript("acct", "chat-1", "Alice", &rows).unwrap();
        let second = memory_doc_ingest_peer_transcript("acct", "chat-2", "Alice", &rows).unwrap();

        assert_eq!(first["key"].as_str(), Some("chat-1:Alice"));
        assert_eq!(second["key"].as_str(), Some("chat-2:Alice"));
    }

    #[test]
    fn short_account_truncates_on_char_boundary() {
        assert_eq!(short_account("acct-123"), "acct-123");
        assert_eq!(short_account("ééééééééé"), "éééééééé");
    }
}
