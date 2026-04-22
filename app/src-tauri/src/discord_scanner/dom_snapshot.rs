//! Discord sidebar scrape via `DOMSnapshot.captureSnapshot`. Replaces the
//! old recipe.js scraper. Discord uses hashed class names (`name__abcde`)
//! so selectors rely on stable ARIA roles + `data-list-item-id`
//! attributes + class-name prefixes.
//!
//!   * rows:  `[role="treeitem"][data-list-item-id]` or
//!     `data-list-item-id^="channels"|"private-channels"`
//!   * name:  class prefix `name_` / `channelName_` / first link text
//!   * badge: class prefix `numberBadge_` / `unread_` / `aria-label*=unread`

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
        let name = find_name(&snap, idx).unwrap_or_default();
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
                "id": format!("dc:{}:{idx}", r.name),
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
    if snap.attr(idx, "role") == Some("treeitem") && snap.attr(idx, "data-list-item-id").is_some() {
        return true;
    }
    if let Some(dlii) = snap.attr(idx, "data-list-item-id") {
        if dlii.starts_with("channels") || dlii.starts_with("private-channels") {
            return true;
        }
    }
    false
}

fn find_name(snap: &Snapshot, root: usize) -> Option<String> {
    if let Some(n) = snap.find_descendant(root, |s, i| {
        s.is_element(i) && s.class_starts_with(i, "name_")
    }) {
        let t = snap.text_content(n);
        if !t.is_empty() {
            return Some(t);
        }
    }
    if let Some(n) = snap.find_descendant(root, |s, i| {
        s.is_element(i) && s.class_starts_with(i, "channelName_")
    }) {
        let t = snap.text_content(n);
        if !t.is_empty() {
            return Some(t);
        }
    }
    // Fallback: first anchor's text.
    let a = snap.find_descendant(root, |s, i| {
        s.is_element(i) && s.tag(i).eq_ignore_ascii_case("A")
    })?;
    let t = snap.text_content(a);
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

fn find_badge(snap: &Snapshot, root: usize) -> Option<u32> {
    // Numeric badge — class prefix `numberBadge_`.
    if let Some(n) = snap.find_descendant(root, |s, i| {
        s.is_element(i) && s.class_starts_with(i, "numberBadge_")
    }) {
        if let Ok(n_parsed) = snap.text_content(n).trim().parse::<u32>() {
            return Some(n_parsed);
        }
    }
    // Generic unread marker — present without a numeric count. Return 1
    // so `totalUnread` increments and the ingest event fires, matching
    // the recipe's `parseInt("") → NaN → 0` behavior (which actually
    // produced 0 there, but the recipe's `unread > 0 ? totalUnread += unread : noop`
    // meant pure-marker rows didn't bump the total — we keep that here).
    if snap
        .find_descendant(root, |s, i| {
            s.is_element(i) && s.class_starts_with(i, "unread_")
        })
        .is_some()
    {
        return Some(0);
    }
    None
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
    for r in rows.iter().take(8) {
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
