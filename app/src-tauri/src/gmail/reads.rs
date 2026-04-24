//! Read ops: list_labels, list_messages, search, get_message.
//!
//! Strategy per-op:
//!
//! * **list_labels** — DOM snapshot of the sidebar. Cheap and reliable:
//!   Gmail renders labels as `<a role="link" aria-label="…">` inside the
//!   left nav. We read that tree directly via
//!   `DOMSnapshot.captureSnapshot` — no JS eval, no network round-trip.
//! * **list_messages / search / get_message** — scaffolded with
//!   structured errors for the first cut. These depend on either
//!   intercepting Gmail's internal batch endpoints via `Network.*` or a
//!   broader DOM walk; both need careful follow-up work to stay stable
//!   across Gmail UI churn. See plan §deferred.
//!
//! Everything here is CEF-only — CDP requires a remote-debugging port
//! which wry doesn't expose.

use super::session;
use super::types::{GmailLabel, GmailMessage};
use crate::cdp::{CdpConn, Snapshot};

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
    _limit: u32,
    _label: Option<String>,
) -> Result<Vec<GmailMessage>, String> {
    log::debug!("[gmail][{account_id}] list_messages (not implemented)");
    Err(format!(
        "gmail[{account_id}]: list_messages not implemented — follow-up work \
         per plan §deferred (Network MITM of mail.google.com sync endpoint)"
    ))
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

pub async fn get_message(
    account_id: &str,
    _message_id: String,
) -> Result<GmailMessage, String> {
    log::debug!("[gmail][{account_id}] get_message (not implemented)");
    Err(format!(
        "gmail[{account_id}]: get_message not implemented — follow-up work"
    ))
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
/// Peel the name off the front and any trailing "N unread"-ish suffix.
fn parse_aria_label(aria: &str) -> (String, Option<u64>) {
    // Try to find a trailing number that looks like an unread count.
    let mut unread: Option<u64> = None;
    let name = aria.trim();
    let lower = name.to_ascii_lowercase();

    // Heuristic: last whitespace-separated token that parses as u64.
    if let Some(last) = name.rsplit_whitespace().next() {
        if let Ok(n) = last.trim_end_matches(',').parse::<u64>() {
            unread = Some(n);
        }
    }
    // Strip common suffixes so the name we surface is just "Inbox" etc.
    let mut name = name.to_string();
    for suf in [
        " unread messages",
        " unread",
        " messages",
        ",",
    ] {
        if name.to_ascii_lowercase().ends_with(suf) {
            let cut = name.len() - suf.len();
            name.truncate(cut);
            name = name.trim_end_matches(',').trim().to_string();
        }
    }
    // If we stripped a count, drop the trailing digits.
    if unread.is_some() {
        // Remove any trailing digits and separator characters.
        while let Some(ch) = name.chars().last() {
            if ch.is_ascii_digit() || ch == ',' || ch == ' ' {
                name.pop();
            } else {
                break;
            }
        }
    }

    let _ = lower; // silence unused when debug build strips logs
    (name.trim().to_string(), unread)
}

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
        assert_eq!(parse_aria_label("Inbox 23 unread"), ("Inbox".into(), Some(23)));
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
