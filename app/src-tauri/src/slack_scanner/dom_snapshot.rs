//! Slack channel-sidebar scrape via `DOMSnapshot.captureSnapshot`. Replaces
//! the old recipe.js scraper. Selectors mirror the old recipe:
//!   * rows:  `[data-qa="virtual-list-item"]` or `.p-channel_sidebar__channel`
//!   * name:  `[data-qa="channel_sidebar_name_button"]` / `.p-channel_sidebar__name` / first `span`
//!   * badge: `.p-channel_sidebar__badge` / `[data-qa="mention_badge"]`

use serde_json::{json, Value};

use crate::cdp::{CdpConn, Snapshot};

#[derive(Debug, Clone)]
pub struct ChannelRow {
    pub name: String,
    pub unread: u32,
}

pub struct DomScan {
    pub rows: Vec<ChannelRow>,
    pub total_unread: u32,
    pub hash: u64,
}

pub async fn scan(cdp: &mut CdpConn, session: &str) -> Result<DomScan, String> {
    let snap = Snapshot::capture(cdp, session).await?;
    let row_nodes = snap.find_all(is_channel_row);
    let mut rows = Vec::with_capacity(row_nodes.len());
    let mut total_unread: u32 = 0;
    for idx in row_nodes {
        let name = find_channel_name(&snap, idx).unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let badge = find_badge(&snap, idx).unwrap_or(0);
        total_unread = total_unread.saturating_add(badge);
        rows.push(ChannelRow {
            name,
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

pub fn ingest_payload(scan: &DomScan) -> Value {
    let messages: Vec<Value> = scan
        .rows
        .iter()
        .enumerate()
        .map(|(idx, r)| {
            json!({
                "id": format!("sl:{}:{idx}", r.name),
                "from": r.name,
                "body": Value::Null,
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

fn is_channel_row(snap: &Snapshot, idx: usize) -> bool {
    if snap.attr(idx, "data-qa") == Some("virtual-list-item") {
        return true;
    }
    snap.has_class(idx, "p-channel_sidebar__channel")
}

fn find_channel_name(snap: &Snapshot, root: usize) -> Option<String> {
    // 1. [data-qa="channel_sidebar_name_button"]
    if let Some(n) = snap.find_descendant(root, |s, i| {
        s.is_element(i) && s.attr(i, "data-qa") == Some("channel_sidebar_name_button")
    }) {
        let t = snap.text_content(n);
        if !t.is_empty() {
            return Some(t);
        }
    }
    // 2. .p-channel_sidebar__name
    if let Some(n) = snap.find_descendant(root, |s, i| {
        s.is_element(i) && s.has_class(i, "p-channel_sidebar__name")
    }) {
        let t = snap.text_content(n);
        if !t.is_empty() {
            return Some(t);
        }
    }
    // 3. first span
    let span = snap.find_descendant(root, |s, i| {
        s.is_element(i) && s.tag(i).eq_ignore_ascii_case("SPAN")
    })?;
    let t = snap.text_content(span);
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

fn find_badge(snap: &Snapshot, root: usize) -> Option<u32> {
    let n = snap.find_descendant(root, |s, i| {
        s.is_element(i)
            && (s.has_class(i, "p-channel_sidebar__badge")
                || s.attr(i, "data-qa") == Some("mention_badge"))
    })?;
    // Matches the Discord scraper: a present-but-empty badge (generic
    // unread marker) returns Some(0) so the row is still included in
    // the ingest, but `total_unread` isn't bumped.
    let txt = snap.text_content(n);
    let trimmed = txt.trim();
    if trimmed.is_empty() {
        return Some(0);
    }
    trimmed.parse::<u32>().ok()
}

fn hash_rows(rows: &[ChannelRow], total_unread: u32) -> u64 {
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
    for r in rows {
        for b in r.name.as_bytes() {
            mix(&mut h, *b);
        }
        mix(&mut h, 0x7c);
        for b in r.unread.to_le_bytes() {
            mix(&mut h, b);
        }
    }
    h
}
