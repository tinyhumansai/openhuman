//! Telegram chat-list scrape via `DOMSnapshot.captureSnapshot`. Replaces
//! the old recipe.js `setInterval` scraper. Pure CDP — no JS runs in the
//! page world.
//!
//! Selectors mirror the old recipe:
//!   * rows:    `.chatlist .chatlist-chat` or `ul.chatlist > li`
//!   * name:    `.user-title` / `.peer-title` / `.dialog-title span`
//!   * preview: `.dialog-subtitle` / `.user-last-message`
//!   * badge:   `.badge-unread` / `.dialog-subtitle-badge-unread`

use serde_json::{json, Value};

use crate::cdp::{CdpConn, Snapshot};

#[derive(Debug, Clone)]
pub struct ChatRow {
    pub name: String,
    pub preview: Option<String>,
    pub unread: u32,
}

pub struct DomScan {
    pub rows: Vec<ChatRow>,
    pub total_unread: u32,
    pub hash: u64,
}

pub async fn scan(cdp: &mut CdpConn, session: &str) -> Result<DomScan, String> {
    let snap = Snapshot::capture(cdp, session).await?;
    let row_nodes = snap.find_all(is_chat_row);
    let mut rows = Vec::with_capacity(row_nodes.len());
    let mut total_unread: u32 = 0;
    for idx in row_nodes {
        let name = find_text_by_class(&snap, idx, &["user-title", "peer-title"])
            .or_else(|| find_dialog_title(&snap, idx))
            .unwrap_or_default();
        let preview = find_text_by_class(&snap, idx, &["dialog-subtitle", "user-last-message"]);
        let badge = find_text_by_class(
            &snap,
            idx,
            &["badge-unread", "dialog-subtitle-badge-unread"],
        )
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0);
        if name.is_empty() && preview.as_deref().map(str::is_empty).unwrap_or(true) {
            continue;
        }
        total_unread = total_unread.saturating_add(badge);
        rows.push(ChatRow {
            name,
            preview,
            unread: badge,
        });
    }
    let hash = hash_rows(&rows, total_unread);
    Ok(DomScan {
        rows,
        total_unread,
        hash,
    })
}

/// Build the ingest-shape payload the React layer already consumes (via
/// `persistIngestToMemory` in `webviewAccountService.ts`). Matches the
/// previous recipe `api.ingest` envelope so no frontend changes required.
pub fn ingest_payload(scan: &DomScan) -> Value {
    let messages: Vec<Value> = scan
        .rows
        .iter()
        .enumerate()
        .map(|(idx, r)| {
            json!({
                "id": if r.name.is_empty() {
                    format!("tg:row:{idx}")
                } else {
                    format!("tg:{}", r.name)
                },
                "from": if r.name.is_empty() { Value::Null } else { json!(r.name) },
                "body": r.preview.clone().map(Value::String).unwrap_or(Value::Null),
                "unread": r.unread,
            })
        })
        .collect();
    let snapshot_key = format!("{:x}", scan.hash);
    json!({
        "messages": messages,
        "unread": scan.total_unread,
        "snapshotKey": snapshot_key,
    })
}

fn is_chat_row(snap: &Snapshot, idx: usize) -> bool {
    if snap.has_class(idx, "chatlist-chat") {
        return true;
    }
    // `ul.chatlist > li` — match `LI` whose parent has class `chatlist`.
    if snap.tag(idx).eq_ignore_ascii_case("LI") {
        // Parent-index walk through the precomputed tree.
        if let Some(parent) = parent_of(snap, idx) {
            if snap.has_class(parent, "chatlist") {
                return true;
            }
        }
    }
    false
}

fn parent_of(snap: &Snapshot, idx: usize) -> Option<usize> {
    (0..snap.len()).find(|&i| snap.children(i).contains(&idx))
}

fn find_text_by_class(snap: &Snapshot, root: usize, classes: &[&str]) -> Option<String> {
    let node = snap.find_descendant(root, |s, i| {
        s.is_element(i) && classes.iter().any(|c| s.has_class(i, c))
    })?;
    let t = snap.text_content(node);
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

fn find_dialog_title(snap: &Snapshot, root: usize) -> Option<String> {
    // `.dialog-title span` — find any descendant `SPAN` whose ancestor has
    // class `dialog-title`. Cheap heuristic: find `.dialog-title` and take
    // its first SPAN descendant's text.
    let container = snap.find_descendant(root, |s, i| {
        s.is_element(i) && s.has_class(i, "dialog-title")
    })?;
    let span = snap.find_descendant(container, |s, i| {
        s.is_element(i) && s.tag(i).eq_ignore_ascii_case("SPAN")
    })?;
    let t = snap.text_content(span);
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

fn hash_rows(rows: &[ChatRow], total_unread: u32) -> u64 {
    // Same fingerprint the recipe used: count, total unread, and the first
    // five rows' (name, body, unread). Tiny FNV-1a over the concatenation.
    let mut h: u64 = 0xcbf29ce484222325;
    fn mix(h: &mut u64, b: u8) {
        *h ^= b as u64;
        *h = h.wrapping_mul(0x100000001b3);
    }
    for b in (rows.len() as u32).to_le_bytes() {
        mix(&mut h, b);
    }
    for b in total_unread.to_le_bytes() {
        mix(&mut h, b);
    }
    for r in rows.iter().take(5) {
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
    h
}
