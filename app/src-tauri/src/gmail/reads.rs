//! Read ops: list_labels, list_messages, search, get_message.
//!
//! Strategy per-op:
//!
//! * **list_labels** — DOM snapshot of the sidebar. Cheap and reliable.
//! * **list_messages** — Gmail's stable Atom feed at
//!   `mail.google.com/mail/u/0/feed/atom[/<label>]`, fetched
//!   authenticated via the attached CDP session (Network.loadNetworkResource
//!   + IO.read — no JS eval). Covers the 20 most recent unread messages.
//! * **search / get_message** — scaffolded with structured errors for
//!   the first cut. Search needs `Page.navigate('#search/<q>')` plus
//!   DOM/Network observation; `get_message` can use Gmail's print-view
//!   endpoint on a per-id basis (follow-up).
//!
//! Everything here is CEF-only — CDP requires a remote-debugging port
//! which wry doesn't expose.

use super::cdp_fetch;
use super::session;
use super::types::{GmailLabel, GmailMessage};
use crate::cdp::Snapshot;
use crate::gmail::atom;

pub async fn list_labels(account_id: &str) -> Result<Vec<GmailLabel>, String> {
    log::debug!("[gmail][{account_id}] list_labels");
    let (mut cdp, session_id) = session::attach(account_id).await?;
    let snap = match Snapshot::capture(&mut cdp, &session_id).await {
        Ok(s) => s,
        Err(e) => {
            session::detach(&mut cdp, &session_id).await;
            return Err(format!("gmail[{account_id}]: snapshot failed: {e}"));
        }
    };
    let labels = scrape_labels(&snap);
    session::detach(&mut cdp, &session_id).await;
    log::debug!(
        "[gmail][{account_id}] list_labels ok count={}",
        labels.len()
    );
    Ok(labels)
}

pub async fn list_messages(
    account_id: &str,
    limit: u32,
    label: Option<String>,
) -> Result<Vec<GmailMessage>, String> {
    log::debug!(
        "[gmail][{account_id}] list_messages limit={limit} label={:?}",
        label
    );
    let url = atom_feed_url(label.as_deref());
    let (mut cdp, session_id) = session::attach(account_id).await?;
    let body = match cdp_fetch::fetch(&mut cdp, &session_id, &url).await {
        Ok(b) => b,
        Err(e) => {
            session::detach(&mut cdp, &session_id).await;
            return Err(format!("gmail[{account_id}]: atom-feed fetch failed: {e}"));
        }
    };
    session::detach(&mut cdp, &session_id).await;
    let mut messages = atom::parse(&body);
    log::debug!(
        "[gmail][{account_id}] list_messages parsed={} (pre-cap)",
        messages.len()
    );
    if (messages.len() as u32) > limit {
        messages.truncate(limit as usize);
    }
    Ok(messages)
}

/// Build the Atom feed URL for a given label. Gmail exposes a default
/// inbox feed at `…/feed/atom` and per-label feeds at
/// `…/feed/atom/<label>`. Unknown labels 404 cleanly, so we don't try
/// to validate here.
fn atom_feed_url(label: Option<&str>) -> String {
    const BASE: &str = "https://mail.google.com/mail/u/0/feed/atom";
    match label {
        None | Some("") | Some("INBOX") | Some("inbox") => BASE.to_string(),
        Some(name) => format!("{BASE}/{}", url_path_escape(name)),
    }
}

/// Minimal path-segment percent-escape for Gmail label names. Gmail
/// allows `/` in user labels (nested), so we only escape the handful
/// of characters that break URL parsing.
fn url_path_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            c if c.is_ascii_alphanumeric() => out.push(c),
            '-' | '_' | '.' | '~' | '/' => out.push(ch),
            other => {
                let mut buf = [0u8; 4];
                for b in other.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("%{:02X}", b));
                }
            }
        }
    }
    out
}

pub async fn search(
    account_id: &str,
    _query: String,
    _limit: u32,
) -> Result<Vec<GmailMessage>, String> {
    log::debug!("[gmail][{account_id}] search (not implemented)");
    Err(format!(
        "gmail[{account_id}]: search not implemented — follow-up work"
    ))
}

pub async fn get_message(account_id: &str, message_id: String) -> Result<GmailMessage, String> {
    log::debug!("[gmail][{account_id}] get_message id={message_id}");
    let url = print_view_url(&message_id);
    let (mut cdp, session_id) = session::attach(account_id).await?;
    let body = match cdp_fetch::fetch(&mut cdp, &session_id, &url).await {
        Ok(b) => b,
        Err(e) => {
            session::detach(&mut cdp, &session_id).await;
            return Err(format!("gmail[{account_id}]: print-view fetch failed: {e}"));
        }
    };
    session::detach(&mut cdp, &session_id).await;
    super::print_view::parse(&message_id, &body)
        .ok_or_else(|| format!("gmail[{account_id}]: print-view parse failed"))
}

/// Gmail's print-view URL — undocumented but stable, returns a clean
/// plain-HTML rendering of a single message/thread with subject/from/
/// to/date/body in a predictable structure.
///
/// Gmail exposes two id formats on this endpoint:
///
/// * Hex thread ids via `th=<hex>` — what the inbox UI uses internally.
/// * Decimal ids via `permthid=thread-f:<dec>&permmsgid=msg-f:<dec>`
///   — this is what the Atom feed gives us.
///
/// We build the decimal form so the id that `list_messages` returns
/// flows directly into `get_message` without conversion.
fn print_view_url(message_id: &str) -> String {
    let escaped = url_path_escape(message_id);
    format!(
        "https://mail.google.com/mail/u/0/?ui=2&view=pt&search=all\
         &permthid=thread-f:{escaped}&permmsgid=msg-f:{escaped}"
    )
}

// ── label scrape ────────────────────────────────────────────────────────

/// Gmail's sidebar labels render as `<a>` or `<div>` with
/// `role="link"` and an `aria-label` attribute containing the label
/// name (and sometimes the unread count). We walk every such node in
/// the snapshot and dedupe by name.
///
/// System labels come in with upper-case English names (Inbox, Sent,
/// Drafts, Spam, Trash, Starred, Important, Snoozed, Scheduled,
/// All Mail, Chats, Categories). Anything else is assumed user-created.
fn scrape_labels(snap: &Snapshot) -> Vec<GmailLabel> {
    let mut out: Vec<GmailLabel> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let link_nodes = snap.find_all(|s, idx| {
        if !s.is_element(idx) {
            return false;
        }
        // Gmail sidebar items are anchors (`<a>`) or `<div role="link">`.
        let tag = s.tag(idx);
        if tag != "A" && tag != "a" && tag != "DIV" && tag != "div" {
            return false;
        }
        matches!(s.attr(idx, "role"), Some("link"))
    });

    for idx in link_nodes {
        let aria = match snap.attr(idx, "aria-label") {
            Some(v) if !v.is_empty() => v,
            _ => continue,
        };
        let (name, unread) = parse_aria_label(aria);
        if name.is_empty() {
            continue;
        }
        if !seen.insert(name.clone()) {
            continue;
        }
        let kind = if is_system_label(&name) {
            "system"
        } else {
            "user"
        };
        out.push(GmailLabel {
            id: name.clone(),
            name,
            kind: kind.to_string(),
            unread,
        });
    }
    out
}

/// Gmail's aria-labels look like:
///   `"Inbox 23 unread"`, `"Inbox, 23 unread messages"`,
///   `"Starred"`, `"Drafts 4"`, `"Spam, 1"`.
/// Peel any trailing `N unread(messages)?` / `N` count off and return
/// the plain label name plus the parsed unread count if present.
fn parse_aria_label(aria: &str) -> (String, Option<u64>) {
    let mut name = aria.trim().to_string();

    // 1. Strip English descriptors in order from most-specific to least.
    //    Keep going until no more of these match, which covers labels
    //    like "Spam, 1 unread messages" that chain two suffixes.
    loop {
        let lower = name.to_ascii_lowercase();
        let stripped_len = ["unread messages", "unread", "messages"]
            .iter()
            .find(|suf| lower.ends_with(*suf))
            .map(|suf| name.len() - suf.len());
        match stripped_len {
            Some(n) => {
                name.truncate(n);
                name = name.trim_end_matches([' ', ',']).to_string();
            }
            None => break,
        }
    }

    // 2. Now name is e.g. "Inbox 23" or "Spam, 1" or "Starred". Peel off
    //    a trailing integer (with any comma/space separator) as the
    //    unread count.
    let mut unread: Option<u64> = None;
    if let Some(last) = name.split(|c: char| c == ' ' || c == ',').next_back() {
        if !last.is_empty() {
            if let Ok(n) = last.parse::<u64>() {
                unread = Some(n);
                let cut = name.len() - last.len();
                name.truncate(cut);
                name = name.trim_end_matches([' ', ',']).to_string();
            }
        }
    }

    (name.trim().to_string(), unread)
}

/// English-only catalog of Gmail's built-in label names. Users on
/// non-English locales will see their labels classified as `"user"`
/// until we switch to a locale-agnostic detector (structural DOM cue
/// or a localised translation table). Tracked as a follow-up in the
/// plan — see `GmailLabel` doc for the caller-facing implication.
fn is_system_label(name: &str) -> bool {
    matches!(
        name,
        "Inbox"
            | "Starred"
            | "Snoozed"
            | "Important"
            | "Sent"
            | "Drafts"
            | "Scheduled"
            | "All Mail"
            | "Spam"
            | "Trash"
            | "Chats"
            | "Categories"
            | "Updates"
            | "Promotions"
            | "Social"
            | "Forums"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_aria_label_peels_trailing_count() {
        assert_eq!(
            parse_aria_label("Inbox 23 unread"),
            ("Inbox".into(), Some(23))
        );
        assert_eq!(parse_aria_label("Drafts 4"), ("Drafts".into(), Some(4)));
        assert_eq!(parse_aria_label("Starred"), ("Starred".into(), None));
        assert_eq!(
            parse_aria_label("Spam, 1 unread messages"),
            ("Spam".into(), Some(1))
        );
    }

    #[test]
    fn system_label_catalog_matches_known_names() {
        for n in ["Inbox", "Sent", "Drafts", "Trash", "Spam", "Starred"] {
            assert!(is_system_label(n), "expected system: {n}");
        }
        assert!(!is_system_label("Receipts"));
        assert!(!is_system_label("Personal/Finance"));
    }
}
