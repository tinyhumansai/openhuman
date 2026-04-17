//! Pure-CDP DOM scrape for WhatsApp message rows.
//!
//! Replaces the old `dom_scan.js` (injected via `Runtime.evaluate`) with a
//! single `DOMSnapshot.captureSnapshot` call that runs at the browser's C++
//! level — no JavaScript executes in the page's JS world. The returned
//! flat-array snapshot is walked in Rust to:
//!
//!   1. locate `[data-id]` elements whose id parses as
//!      `"<fromMe>_<chatId>_<msgId>"` (message rows),
//!   2. pull `data-pre-plain-text` off a descendant to recover author +
//!      timestamp,
//!   3. collect rendered body text from descendant
//!      `span.selectable-text` / `span[dir="ltr|rtl"]` nodes.
//!
//! Output matches the shape `dom_scan.js` used to return so the rest of
//! the scanner (merge, emit, hash-dedup) doesn't need to change.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use serde_json::{json, Value};

use super::CdpConn;

/// One scraped message row. Mirrors the JSON object the old JS emitted so
/// the merge path in `mod.rs` keeps working unchanged.
#[derive(Debug, Clone)]
pub struct DomMessage {
    pub data_id: String,
    pub from_me: bool,
    pub chat_id: String,
    pub msg_id: String,
    pub author: Option<String>,
    pub pre_timestamp: Option<String>,
    pub body: String,
}

impl DomMessage {
    pub fn to_json(&self) -> Value {
        json!({
            "dataId": self.data_id,
            "fromMe": self.from_me,
            "chatId": self.chat_id,
            "msgId": self.msg_id,
            "author": self.author,
            "preTimestamp": self.pre_timestamp,
            "body": self.body,
        })
    }
}

/// Run `DOMSnapshot.captureSnapshot` against an attached page session and
/// return parsed message rows + a FNV-1a hash over (dataId, body) so callers
/// can skip emission when nothing changed.
pub async fn capture_messages(
    cdp: &mut CdpConn,
    session: &str,
) -> Result<(Vec<DomMessage>, u64), String> {
    // `computedStyles` is a required array — empty is fine, we don't need
    // any CSS. The other flags default sensibly; explicitly disable the
    // heavy paint/rect output to keep payloads small.
    let raw = cdp
        .call(
            "DOMSnapshot.captureSnapshot",
            json!({
                "computedStyles": [],
                "includePaintOrder": false,
                "includeDOMRects": false,
            }),
            Some(session),
        )
        .await?;
    let snap: CaptureSnapshot =
        serde_json::from_value(raw).map_err(|e| format!("decode DOMSnapshot: {e}"))?;
    let rows = parse_rows(&snap);
    let hash = fnv_hash(&rows);
    Ok((rows, hash))
}

// ─── CDP response shape ─────────────────────────────────────────────

#[derive(Deserialize, Debug, Default)]
struct CaptureSnapshot {
    #[serde(default)]
    documents: Vec<DocumentSnap>,
    #[serde(default)]
    strings: Vec<String>,
}

#[derive(Deserialize, Debug, Default)]
struct DocumentSnap {
    #[serde(default)]
    nodes: NodeTreeSnap,
}

/// Flat-array node tree from `DOMSnapshot.NodeTreeSnapshot`. Each array is
/// indexed by node index; -1 sentinel means "absent". `attributes[i]` is a
/// flat list of alternating `[nameIdx, valueIdx, ...]` string-table indices.
#[derive(Deserialize, Debug, Default)]
struct NodeTreeSnap {
    #[serde(rename = "parentIndex", default)]
    parent_index: Vec<i32>,
    #[serde(rename = "nodeType", default)]
    node_type: Vec<i32>,
    #[serde(rename = "nodeName", default)]
    node_name: Vec<i32>,
    #[serde(rename = "nodeValue", default)]
    node_value: Vec<i32>,
    #[serde(default)]
    attributes: Vec<Vec<i32>>,
}

const NODE_TYPE_ELEMENT: i32 = 1;
const NODE_TYPE_TEXT: i32 = 3;
/// Hard cap on body length to mirror `dom_scan.js` (which sliced at 4000).
const MAX_BODY_CHARS: usize = 4000;

// ─── parsing ────────────────────────────────────────────────────────

fn parse_rows(snap: &CaptureSnapshot) -> Vec<DomMessage> {
    // Main frame only — iframes aren't used by WhatsApp's message list.
    let doc = match snap.documents.first() {
        Some(d) => d,
        None => return Vec::new(),
    };
    let nodes = &doc.nodes;
    let strings = &snap.strings;
    let count = nodes.node_type.len();
    if count == 0 {
        return Vec::new();
    }

    // Precompute children map so descendant walks are O(subtree) instead of
    // O(total-nodes) per root.
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (i, &p) in nodes.parent_index.iter().enumerate() {
        if p >= 0 && (p as usize) < count {
            children[p as usize].push(i);
        }
    }

    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for i in 0..count {
        if nodes.node_type.get(i).copied().unwrap_or(0) != NODE_TYPE_ELEMENT {
            continue;
        }
        let attrs = attrs_map(nodes, i, strings);
        let data_id = match attrs.get("data-id") {
            Some(v) if !v.is_empty() => v.clone(),
            _ => continue,
        };
        // data-id format: "<fromMe>_<chatId>_<msgId>" — chat-list rows and
        // other framework hooks use different shapes, so filter strictly.
        let (from_me, chat_id, msg_id) = match split_data_id(&data_id) {
            Some(x) => x,
            None => continue,
        };
        if !seen.insert(data_id.clone()) {
            continue;
        }

        let (pre_ts, author) = find_pre_plain(nodes, strings, &children, i);
        let body = find_body(nodes, strings, &children, i);
        // A row with neither a body nor a pre-plain-text tag is just chrome
        // (avatar wrapper, reaction chip, etc) — skip it.
        if body.is_empty() && pre_ts.is_none() {
            continue;
        }

        out.push(DomMessage {
            data_id,
            from_me,
            chat_id,
            msg_id,
            author,
            pre_timestamp: pre_ts,
            body: truncate_chars(&body, MAX_BODY_CHARS),
        });
    }

    out
}

/// Build a `name → value` map for a single element's attributes. Missing or
/// malformed entries are silently skipped.
fn attrs_map(nodes: &NodeTreeSnap, idx: usize, strings: &[String]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Some(flat) = nodes.attributes.get(idx) {
        let mut i = 0;
        while i + 1 < flat.len() {
            let k = str_at(strings, flat[i]);
            let v = str_at(strings, flat[i + 1]);
            if !k.is_empty() {
                map.insert(k.to_string(), v.to_string());
            }
            i += 2;
        }
    }
    map
}

fn str_at(strings: &[String], idx: i32) -> &str {
    if idx < 0 {
        return "";
    }
    strings.get(idx as usize).map(String::as_str).unwrap_or("")
}

/// Parse `"true_12345@c.us_3EB0A..."` → `(true, "12345@c.us", "3EB0A...")`.
fn split_data_id(s: &str) -> Option<(bool, String, String)> {
    // `splitn(3, '_')` keeps the msgId intact even when it contains `_`.
    let mut it = s.splitn(3, '_');
    let from_me_tok = it.next()?;
    let chat_id = it.next()?;
    let msg_id = it.next()?;
    let from_me = match from_me_tok {
        "true" => true,
        "false" => false,
        _ => return None,
    };
    if chat_id.is_empty() || msg_id.is_empty() {
        return None;
    }
    Some((from_me, chat_id.to_string(), msg_id.to_string()))
}

/// Find the first descendant carrying `data-pre-plain-text` and parse
/// `"[HH:MM, D/M/YYYY] Author Name: "` out of it.
fn find_pre_plain(
    nodes: &NodeTreeSnap,
    strings: &[String],
    children: &[Vec<usize>],
    root: usize,
) -> (Option<String>, Option<String>) {
    let mut stack = vec![root];
    while let Some(idx) = stack.pop() {
        if nodes.node_type.get(idx).copied().unwrap_or(0) == NODE_TYPE_ELEMENT {
            if let Some(flat) = nodes.attributes.get(idx) {
                let mut i = 0;
                while i + 1 < flat.len() {
                    if str_at(strings, flat[i]) == "data-pre-plain-text" {
                        let pre = str_at(strings, flat[i + 1]);
                        if let Some(parsed) = parse_pre_attr(pre) {
                            return (Some(parsed.0), Some(parsed.1));
                        }
                    }
                    i += 2;
                }
            }
        }
        if let Some(kids) = children.get(idx) {
            // Depth-first, preserve order — doesn't matter for correctness
            // but keeps behavior predictable when multiple descendants carry
            // the attr (shouldn't happen in WhatsApp's DOM).
            for &k in kids.iter().rev() {
                stack.push(k);
            }
        }
    }
    (None, None)
}

/// Pick the longest rendered body text inside the row. WhatsApp puts each
/// message's text in a descendant `span.selectable-text` or a
/// `span[dir="ltr|rtl"]`; walking every such span and keeping the longest
/// one matches `dom_scan.js`. Falls back to the row's full text with the
/// "[HH:MM, D/M/YYYY] Author:" prefix stripped.
fn find_body(
    nodes: &NodeTreeSnap,
    strings: &[String],
    children: &[Vec<usize>],
    root: usize,
) -> String {
    let mut best = String::new();
    let mut stack = vec![root];
    while let Some(idx) = stack.pop() {
        if nodes.node_type.get(idx).copied().unwrap_or(0) == NODE_TYPE_ELEMENT {
            let name = str_at(strings, *nodes.node_name.get(idx).unwrap_or(&-1));
            if name.eq_ignore_ascii_case("SPAN") {
                let attrs = attrs_map(nodes, idx, strings);
                let has_class = attrs
                    .get("class")
                    .map(|c| c.split_whitespace().any(|w| w == "selectable-text"))
                    .unwrap_or(false);
                let dir = attrs.get("dir").map(String::as_str).unwrap_or("");
                if has_class || dir == "ltr" || dir == "rtl" {
                    let t = collect_text(nodes, strings, children, idx);
                    let trimmed = t.trim();
                    if trimmed.len() > best.len() {
                        best = trimmed.to_string();
                    }
                }
            }
        }
        if let Some(kids) = children.get(idx) {
            for &k in kids.iter().rev() {
                stack.push(k);
            }
        }
    }
    if !best.is_empty() {
        return best;
    }
    // Fallback: everything under the row, with the "[HH:MM, ...] Author:"
    // prefix stripped — handles rows rendered without a dedicated text span.
    let full = collect_text(nodes, strings, children, root);
    strip_pre_prefix(full.trim()).to_string()
}

/// Concatenate every TEXT_NODE nodeValue under `root` in document order.
fn collect_text(
    nodes: &NodeTreeSnap,
    strings: &[String],
    children: &[Vec<usize>],
    root: usize,
) -> String {
    let mut out = String::new();
    let mut stack = vec![root];
    while let Some(idx) = stack.pop() {
        if nodes.node_type.get(idx).copied().unwrap_or(0) == NODE_TYPE_TEXT {
            out.push_str(str_at(strings, *nodes.node_value.get(idx).unwrap_or(&-1)));
        }
        if let Some(kids) = children.get(idx) {
            // Reverse so the first child is processed first (stack ordering).
            for &k in kids.iter().rev() {
                stack.push(k);
            }
        }
    }
    out
}

/// Parse `"[12:34, 3/15/2025] John Doe: "` → `("12:34, 3/15/2025", "John Doe")`.
fn parse_pre_attr(pre: &str) -> Option<(String, String)> {
    let s = pre.trim_start();
    if !s.starts_with('[') {
        return None;
    }
    let close = s.find(']')?;
    let ts = s[1..close].trim().to_string();
    let rest = s[close + 1..].trim_start();
    let colon = rest.find(':')?;
    let author = rest[..colon].trim().to_string();
    if ts.is_empty() || author.is_empty() {
        return None;
    }
    Some((ts, author))
}

/// Strip a leading `"[...] foo: "` prefix from a concatenated row text.
fn strip_pre_prefix(text: &str) -> &str {
    let t = text.trim_start();
    if !t.starts_with('[') {
        return text;
    }
    let close = match t.find(']') {
        Some(i) => i,
        None => return text,
    };
    let rest = &t[close + 1..];
    let colon = match rest.find(':') {
        Some(i) => i,
        None => return text,
    };
    let after = &rest[colon + 1..];
    after.strip_prefix(' ').unwrap_or(after)
}

/// Truncate a String to at most `max` chars (not bytes) — safe for UTF-8.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

/// FNV-1a 32-bit rolling hash over `(dataId + 0x01 + body)` per row. Used
/// purely for change detection on the Rust side — no persistence, no wire
/// format. Byte-based (JS version was UTF-16 code units; ASCII-equivalent).
fn fnv_hash(rows: &[DomMessage]) -> u64 {
    let mut h: u32 = 2166136261;
    for r in rows {
        for b in r.data_id.as_bytes() {
            h ^= *b as u32;
            h = h.wrapping_mul(16777619);
        }
        h ^= 0x01;
        h = h.wrapping_mul(16777619);
        for b in r.body.as_bytes() {
            h ^= *b as u32;
            h = h.wrapping_mul(16777619);
        }
    }
    h as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_data_id_parses_msg_row() {
        let (fm, chat, msg) = split_data_id("false_12345@c.us_3EB0ABCDEF").unwrap();
        assert!(!fm);
        assert_eq!(chat, "12345@c.us");
        assert_eq!(msg, "3EB0ABCDEF");
    }

    #[test]
    fn split_data_id_keeps_underscores_in_msg_id() {
        let (_, _, msg) = split_data_id("true_chat@g.us_AB_CD_EF").unwrap();
        assert_eq!(msg, "AB_CD_EF");
    }

    #[test]
    fn split_data_id_rejects_non_message_rows() {
        assert!(split_data_id("chat-list-item_abc").is_none());
        assert!(split_data_id("maybe_abc_def").is_none());
    }

    #[test]
    fn parse_pre_attr_extracts_ts_and_author() {
        let (ts, author) = parse_pre_attr("[4:53 AM, 7/5/2025] Jane Doe: ").unwrap();
        assert_eq!(ts, "4:53 AM, 7/5/2025");
        assert_eq!(author, "Jane Doe");
    }

    #[test]
    fn parse_pre_attr_rejects_malformed() {
        assert!(parse_pre_attr("no bracket").is_none());
        assert!(parse_pre_attr("[only-ts]").is_none());
    }

    #[test]
    fn strip_pre_prefix_drops_leading_meta() {
        assert_eq!(
            strip_pre_prefix("[12:34, 3/15/2025] Bob: hello world"),
            "hello world"
        );
    }

    #[test]
    fn strip_pre_prefix_passthrough_when_no_match() {
        assert_eq!(strip_pre_prefix("hello world"), "hello world");
    }

    #[test]
    fn truncate_chars_is_utf8_safe() {
        // Each emoji is a single char but 4 bytes in UTF-8.
        let s = "💬💬💬💬💬";
        assert_eq!(truncate_chars(s, 3), "💬💬💬");
        assert_eq!(truncate_chars(s, 10), s);
    }
}
