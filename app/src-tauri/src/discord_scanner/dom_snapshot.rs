//! Discord sidebar scrape via `DOMSnapshot.captureSnapshot`. Replaces the
//! old recipe.js scraper. Discord uses hashed class names (`name__abcde`)
//! so selectors rely on stable ARIA roles + `data-list-item-id`
//! attributes + class-name prefixes.
//!
//!   * rows:  `[role="treeitem"][data-list-item-id]` or
//!     `data-list-item-id^="channels"|"private-channels"`
//!   * name:  class prefix `name_` / `channelName_` / first link text
//!   * badge: class prefix `numberBadge_` / `unread_` / `aria-label*=unread`
//!
//! Voice channel detection:
//!   * `[class*="voiceConnected_"]`   — active voice session panel
//!   * `[class*="activityPanel_"]`    — activity panel shown when in VC
//!   * `[class*="connection_"]` with inner "Voice Connected" text

use serde_json::{json, Value};

use crate::cdp::{CdpConn, Snapshot};

#[derive(Debug, Clone)]
pub struct ChannelRow {
    pub name: String,
    pub unread: u32,
}

/// Discord voice-channel call detection result.
#[derive(Debug, Clone, Default)]
pub struct VoiceState {
    /// Whether a voice channel session is active.
    pub active: bool,
    /// Channel name, if detectable.
    pub channel_name: Option<String>,
    /// Guild (server) name, if detectable.
    pub guild_name: Option<String>,
}

pub struct DomScan {
    pub rows: Vec<ChannelRow>,
    pub total_unread: u32,
    pub hash: u64,
    /// Voice/call state at the time of this scan.
    pub voice: VoiceState,
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
    let voice = detect_voice(&snap);
    Ok(DomScan {
        rows,
        total_unread,
        hash,
        voice,
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
    let Some(dlii) = snap.attr(idx, "data-list-item-id") else {
        return false;
    };
    // Primary: any treeitem carrying a list-item id (current Discord DOM).
    // Fallback: legacy rows without `role` but with a well-known id prefix.
    snap.attr(idx, "role") == Some("treeitem")
        || dlii.starts_with("channels")
        || dlii.starts_with("private-channels")
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
    // Pure marker (no numeric count): row is included in `rows` with
    // unread=0 but `total_unread` is not incremented.
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

/// Detect whether a Discord voice channel session is active.
///
/// Discord uses hashed CSS class names, so we match by prefix:
///   - `voiceConnected_` — the bottom panel shown when connected to a VC
///   - `activityPanel_`  — activity panel that appears during VC
///   - `connection_` with "Voice Connected" text inside
fn detect_voice(snap: &Snapshot) -> VoiceState {
    // 1. Voice-connected indicator panel.
    let voice_nodes = snap.find_all(|s, i| {
        s.is_element(i)
            && (s.class_starts_with(i, "voiceConnected_")
                || s.class_starts_with(i, "activityPanel_"))
    });

    if !voice_nodes.is_empty() {
        // Try to find the channel name from a nearby element.
        let channel_name = voice_nodes.first().and_then(|&root| {
            let n = snap.find_descendant(root, |s, i| {
                s.is_element(i)
                    && (s.class_starts_with(i, "channelName_") || s.class_starts_with(i, "name_"))
            })?;
            let t = snap.text_content(n);
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        });

        log::debug!(
            "[dc-dom] voice detected active=true channel_name={:?}",
            channel_name
        );
        return VoiceState {
            active: true,
            channel_name,
            guild_name: None,
        };
    }

    // 2. Look for "Voice Connected" text in a connection_ panel (fallback).
    let connection_nodes =
        snap.find_all(|s, i| s.is_element(i) && s.class_starts_with(i, "connection_"));
    for &root in &connection_nodes {
        let text = snap.text_content(root);
        if text.to_lowercase().contains("voice connected") {
            log::debug!("[dc-dom] voice detected via 'Voice Connected' text");
            return VoiceState {
                active: true,
                channel_name: None,
                guild_name: None,
            };
        }
    }

    VoiceState::default()
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
