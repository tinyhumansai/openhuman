//! WeChat Web DOM scrape via `DOMSnapshot.captureSnapshot` (pure CDP).

use serde_json::Value;

use crate::cdp::{CdpConn, Snapshot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatRow {
    pub name: String,
    pub preview: Option<String>,
    pub unread: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRow {
    pub chat_id: String,
    pub chat_name: String,
    pub sender: Option<String>,
    pub body: String,
    pub ts: Option<i64>,
}

pub struct DomScan {
    pub chat_rows: Vec<ChatRow>,
    pub messages: Vec<MessageRow>,
    pub active_chat_name: Option<String>,
    pub unread: u32,
    pub hash: u64,
}

pub async fn scan(cdp: &mut CdpConn, session: &str) -> Result<DomScan, String> {
    let snap = Snapshot::capture(cdp, session).await?;
    let mut chat_rows = Vec::new();
    let mut unread: u32 = 0;
    for idx in snap.find_all(is_chat_list_row) {
        let name = find_row_title(&snap, idx).unwrap_or_default();
        let preview = find_row_preview(&snap, idx);
        let badge = find_row_unread(&snap, idx);
        if name.is_empty() && preview.as_deref().map(str::is_empty).unwrap_or(true) {
            continue;
        }
        unread = unread.saturating_add(badge);
        chat_rows.push(ChatRow {
            name,
            preview,
            unread: badge,
        });
    }
    let active_chat_name = find_active_chat_title(&snap);
    let chat_id_base = active_chat_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("active");
    let mut messages = Vec::new();
    for idx in snap.find_all(is_message_bubble) {
        let body = snap.text_content(idx);
        if body.len() < 2 {
            continue;
        }
        messages.push(MessageRow {
            chat_id: chat_id_base.to_string(),
            chat_name: active_chat_name
                .clone()
                .unwrap_or_else(|| chat_id_base.to_string()),
            sender: find_message_sender(&snap, idx),
            body,
            ts: None,
        });
    }
    let hash = hash_scan(&chat_rows, &messages, unread);
    Ok(DomScan {
        chat_rows,
        messages,
        active_chat_name,
        unread,
        hash,
    })
}

pub fn scan_to_core_payload(
    account_id: &str,
    scan: &DomScan,
) -> openhuman_core::openhuman::webview_accounts::WechatScanPayload {
    use openhuman_core::openhuman::webview_accounts::{
        WechatChatRow, WechatMessageRow, WechatScanPayload,
    };
    WechatScanPayload {
        account_id: account_id.to_string(),
        chat_rows: scan
            .chat_rows
            .iter()
            .map(|r| WechatChatRow {
                name: r.name.clone(),
                preview: r.preview.clone(),
                unread: r.unread,
            })
            .collect(),
        messages: scan
            .messages
            .iter()
            .map(|m| WechatMessageRow {
                chat_id: m.chat_id.clone(),
                chat_name: m.chat_name.clone(),
                sender: m.sender.clone(),
                body: m.body.clone(),
                ts: m.ts,
            })
            .collect(),
        unread: scan.unread,
        snapshot_key: format!("{:x}", scan.hash),
        source: "cdp-dom".to_string(),
    }
}

#[allow(dead_code)]
pub fn ingest_payload_for_scan(scan: &DomScan) -> Value {
    openhuman_core::openhuman::webview_accounts::list_ingest_payload(&scan_to_core_payload(
        "test-account",
        scan,
    ))
}

fn is_chat_list_row(snap: &Snapshot, idx: usize) -> bool {
    if !snap.is_element(idx) {
        return false;
    }
    let tag = snap.tag(idx);
    (tag.eq_ignore_ascii_case("LI") || tag.eq_ignore_ascii_case("DIV"))
        && (class_matches_any(
            snap,
            idx,
            &[
                "session",
                "chat-item",
                "chat_item",
                "conversation-item",
                "recent",
                "nav-item",
            ],
        ) || snap.attr(idx, "data-chat-id").is_some())
}

fn is_message_bubble(snap: &Snapshot, idx: usize) -> bool {
    snap.is_element(idx)
        && (class_matches_any(
            snap,
            idx,
            &[
                "message",
                "msg",
                "bubble",
                "chat-message",
                "message-item",
                "msg-item",
            ],
        ) || snap.attr(idx, "data-message-id").is_some())
}

fn class_matches_any(snap: &Snapshot, idx: usize, needles: &[&str]) -> bool {
    snap.classes(idx).any(|c| {
        let lower = c.to_ascii_lowercase();
        needles.iter().any(|n| lower.contains(n))
    })
}

fn find_row_title(snap: &Snapshot, root: usize) -> Option<String> {
    find_text_by_class_hints(
        snap,
        root,
        &[
            "nickname",
            "nick-name",
            "title",
            "name",
            "user-name",
            "session-name",
        ],
    )
}

fn find_row_preview(snap: &Snapshot, root: usize) -> Option<String> {
    find_text_by_class_hints(
        snap,
        root,
        &["preview", "last-msg", "msg-preview", "desc", "subtitle"],
    )
}

fn find_row_unread(snap: &Snapshot, root: usize) -> u32 {
    find_text_by_class_hints(snap, root, &["badge", "unread", "count", "num"])
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn find_active_chat_title(snap: &Snapshot) -> Option<String> {
    snap.find_all(|s, i| {
        s.is_element(i)
            && class_matches_any(
                s,
                i,
                &["chat-title", "conversation-title", "header-title", "title"],
            )
    })
    .into_iter()
    .find_map(|idx| {
        let t = snap.text_content(idx);
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    })
}

fn find_message_sender(snap: &Snapshot, bubble: usize) -> Option<String> {
    parent_of(snap, bubble)
        .and_then(|parent| find_text_by_class_hints(snap, parent, &["sender", "nickname", "name"]))
}

fn find_text_by_class_hints(snap: &Snapshot, root: usize, hints: &[&str]) -> Option<String> {
    let node = snap.find_descendant(root, |s, i| {
        s.is_element(i) && class_matches_any(s, i, hints)
    })?;
    let t = snap.text_content(node);
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

fn parent_of(snap: &Snapshot, idx: usize) -> Option<usize> {
    (0..snap.len()).find(|&i| snap.children(i).contains(&idx))
}

fn hash_scan(chat_rows: &[ChatRow], messages: &[MessageRow], unread: u32) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    fn mix(h: &mut u64, b: u8) {
        *h ^= b as u64;
        *h = h.wrapping_mul(0x100000001b3);
    }
    for b in (chat_rows.len() as u32).to_le_bytes() {
        mix(&mut h, b);
    }
    for b in (messages.len() as u32).to_le_bytes() {
        mix(&mut h, b);
    }
    for b in unread.to_le_bytes() {
        mix(&mut h, b);
    }
    for r in chat_rows {
        for b in r.name.as_bytes() {
            mix(&mut h, *b);
        }
        mix(&mut h, 0x7c);
        if let Some(p) = &r.preview {
            for b in p.as_bytes() {
                mix(&mut h, *b);
            }
        }
        mix(&mut h, 0x7c);
        for b in r.unread.to_le_bytes() {
            mix(&mut h, b);
        }
    }
    for m in messages {
        for b in m.chat_id.as_bytes() {
            mix(&mut h, *b);
        }
        mix(&mut h, 0x7c);
        for b in m.chat_name.as_bytes() {
            mix(&mut h, *b);
        }
        mix(&mut h, 0x7c);
        if let Some(sender) = &m.sender {
            for b in sender.as_bytes() {
                mix(&mut h, *b);
            }
        }
        mix(&mut h, 0x7c);
        for b in m.body.as_bytes() {
            mix(&mut h, *b);
        }
        mix(&mut h, 0x7c);
        if let Some(ts) = m.ts {
            for b in ts.to_le_bytes() {
                mix(&mut h, b);
            }
        }
        mix(&mut h, 0x7c);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_changes_when_message_body_changes() {
        let row = ChatRow {
            name: "A".into(),
            preview: None,
            unread: 0,
        };
        let first = MessageRow {
            chat_id: "c".into(),
            chat_name: "A".into(),
            sender: Some("alice".into()),
            body: "hello".into(),
            ts: Some(1),
        };
        let second = MessageRow {
            body: "world".into(),
            ..first.clone()
        };
        assert_ne!(
            hash_scan(&[row.clone()], &[first], 0),
            hash_scan(&[row], &[second], 0)
        );
    }

    #[test]
    fn hash_changes_when_unread_moves_between_chats() {
        let a1 = ChatRow {
            name: "A".into(),
            preview: None,
            unread: 2,
        };
        let b1 = ChatRow {
            name: "B".into(),
            preview: None,
            unread: 0,
        };
        let a2 = ChatRow {
            name: "A".into(),
            preview: None,
            unread: 0,
        };
        let b2 = ChatRow {
            name: "B".into(),
            preview: None,
            unread: 2,
        };
        assert_ne!(
            hash_scan(&[a1, b1], &[], 2),
            hash_scan(&[a2, b2], &[], 2),
            "per-chat unread distribution must affect the hash"
        );
    }
}
