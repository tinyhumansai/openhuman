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

/// Run a Gmail search by driving the live search input via CDP
/// `Input.dispatchMouseEvent` / `Input.dispatchKeyEvent`. No page JS is
/// executed — we locate the search box from the DOM snapshot, click its
/// centre, type the query, press Enter, then poll the snapshot until
/// the result list materialises.
///
/// The returned `GmailMessage` rows carry the Gmail thread id (decimal
/// `permthid`) plus any subject / snippet / from we were able to scrape
/// from the row. Bodies are NOT populated — callers feed `id` into
/// [`get_message`] (which uses the `print_view` URL pattern) for full
/// text.
///
/// The webview must already be at `https://mail.google.com/mail/u/0/`
/// (the default landing surface after `webview_account_open("gmail")`).
/// If Gmail redirects to `accounts.google.com` we attach in fallback
/// mode and the search input lookup will fail with a clear error.
pub async fn search(
    account_id: &str,
    query: String,
    limit: u32,
) -> Result<Vec<GmailMessage>, String> {
    log::info!("[gmail][{account_id}] search query={:?} limit={}", query, limit);
    let limit = if limit == 0 { 10 } else { limit.min(50) };

    let (mut cdp, session) = session::attach(account_id).await?;

    let outcome = run_search(&mut cdp, &session, account_id, &query, limit as usize).await;
    session::detach(&mut cdp, &session).await;
    outcome
}

async fn run_search(
    cdp: &mut crate::cdp::CdpConn,
    session: &str,
    account_id: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<GmailMessage>, String> {
    use serde_json::json;

    // Two-step navigate to dodge the previous-search-rows-still-rendered
    // race. Gmail's SPA hash router doesn't tear the inbox table down
    // immediately on hash change — when we previously searched
    // `from:linkedin.com` and now search `from:invitations-noreply@…`,
    // for a beat the DOM still shows the old rows. Polling early
    // would scrape those stale rows.
    //
    // Hop through `#inbox` first: that triggers the panel teardown,
    // then we navigate to the actual search URL. We also pre-sleep
    // 1.2 s after the search navigate before the first snapshot so
    // Gmail has time to populate the new results table.
    let inbox = "https://mail.google.com/mail/u/0/#inbox";
    log::info!("[gmail][{account_id}] search clearing via {}", inbox);
    cdp.call("Page.navigate", json!({ "url": inbox }), Some(session))
        .await
        .map_err(|e| format!("gmail[{account_id}] search: clear navigate: {e}"))?;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let target = format!(
        "https://mail.google.com/mail/u/0/#search/{}",
        url_path_escape(query)
    );
    log::info!("[gmail][{account_id}] search navigating to {}", target);
    cdp.call("Page.navigate", json!({ "url": target }), Some(session))
        .await
        .map_err(|e| format!("gmail[{account_id}] search: Page.navigate: {e}"))?;
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;

    // Poll the snapshot until the result list materialises. Gmail's
    // SPA rerender after a hash navigation takes 0.5–2 s; we cap at
    // ~6 s so a network-slow user doesn't stall onboarding forever.
    let mut messages: Vec<GmailMessage> = Vec::new();
    for attempt in 0..15 {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let snap = crate::cdp::Snapshot::capture(cdp, session)
            .await
            .map_err(|e| format!("gmail[{account_id}] search: re-snapshot failed: {e}"))?;
        messages = scrape_search_results(&snap, limit);
        log::debug!(
            "[gmail][{account_id}] search attempt={} rows={}",
            attempt,
            messages.len()
        );
        if !messages.is_empty() {
            break;
        }
    }

    log::info!(
        "[gmail][{account_id}] search ok query={:?} rows={}",
        query,
        messages.len()
    );
    Ok(messages)
}

/// Locate Gmail's search input in a DOM snapshot.
///
/// Match strategy (most-specific first so accidental matches against
/// other inputs in the page are unlikely):
///   1. `<input>` with `aria-label="Search mail"` (English Gmail).
///   2. `<input name="q">` inside `role="search"` form.
///   3. Any `<input>` whose `aria-label` lowercases to contain "search".
fn find_search_input(snap: &crate::cdp::Snapshot) -> Option<usize> {
    use crate::cdp::Snapshot as S;
    if let Some(idx) = snap.find_descendant(0, |s: &S, i| {
        s.is_element(i)
            && eq_ignore_case(s.tag(i), "input")
            && s.attr(i, "aria-label") == Some("Search mail")
    }) {
        return Some(idx);
    }
    if let Some(idx) = snap.find_descendant(0, |s: &S, i| {
        s.is_element(i) && eq_ignore_case(s.tag(i), "input") && s.attr(i, "name") == Some("q")
    }) {
        return Some(idx);
    }
    snap.find_descendant(0, |s: &S, i| {
        if !s.is_element(i) || !eq_ignore_case(s.tag(i), "input") {
            return false;
        }
        s.attr(i, "aria-label")
            .map(|v| v.to_ascii_lowercase().contains("search"))
            .unwrap_or(false)
    })
}

fn eq_ignore_case(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Walk the snapshot for thread rows in Gmail's search result table.
///
/// Modern Gmail tags each thread row as `<tr class="zA">` (plus a few
/// state classes like `zE` for unread, `yO` for inbox, …). The
/// thread-id attributes (`data-legacy-thread-id`, `data-thread-id`,
/// `data-message-id`, `data-thread-perm-id`) sit on descendant
/// elements — usually the subject `<span class="bog">` or a sibling
/// `<span>`, NOT on the `<tr>` itself. We walk inside each `tr.zA` to
/// pick whichever id-bearing descendant we find first.
///
/// Returns rows in document order, capped at `limit`.
fn scrape_search_results(snap: &crate::cdp::Snapshot, limit: usize) -> Vec<GmailMessage> {
    let mut out: Vec<GmailMessage> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let rows = snap.find_all(|s, i| {
        eq_ignore_case(s.tag(i), "tr") && s.classes(i).any(|c| c == "zA")
    });
    for row in rows {
        let Some(id) = scrape_row_thread_id(snap, row) else { continue };
        if id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        out.push(GmailMessage {
            id: id.clone(),
            thread_id: Some(id),
            from: scrape_row_from(snap, row),
            to: Vec::new(),
            cc: Vec::new(),
            subject: scrape_row_subject(snap, row),
            snippet: scrape_row_snippet(snap, row),
            body: None,
            date_ms: None,
            labels: Vec::new(),
            unread: false,
        });
        if out.len() >= limit {
            return out;
        }
    }

    // Fallback when the `tr.zA` shape isn't in use: scan anchors for
    // `permthid=thread-f:<digits>` (older Gmail/print-view bundle).
    if out.is_empty() {
        let anchors = snap.find_all(|s, i| {
            eq_ignore_case(s.tag(i), "a") && s.attr(i, "href").is_some()
        });
        for idx in anchors {
            let href = snap.attr(idx, "href").unwrap_or("");
            let Some(id) = extract_permthid(href) else { continue };
            if !seen.insert(id.clone()) {
                continue;
            }
            out.push(GmailMessage {
                id: id.clone(),
                thread_id: Some(id),
                from: None,
                to: Vec::new(),
                cc: Vec::new(),
                subject: None,
                snippet: Some(snap.text_content(idx)),
                body: None,
                date_ms: None,
                labels: Vec::new(),
                unread: false,
            });
            if out.len() >= limit {
                return out;
            }
        }
    }

    out
}

/// Find a thread-id attribute on `row` or any of its descendants.
/// Tries the modern Gmail attribute names in priority order; the
/// `data-legacy-thread-id` value is the decimal `thread-f:` id format
/// that `print_view_url` accepts directly.
fn scrape_row_thread_id(snap: &crate::cdp::Snapshot, row: usize) -> Option<String> {
    const ATTRS: &[&str] = &[
        "data-legacy-thread-id",
        "data-thread-perm-id",
        "data-thread-id",
        "data-message-id",
        "data-legacy-message-id",
    ];
    for &attr in ATTRS {
        if let Some(v) = snap.attr(row, attr) {
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    let descendant = snap.find_descendant(row, |s, i| {
        if !s.is_element(i) {
            return false;
        }
        ATTRS.iter().any(|a| s.attr(i, a).is_some())
    })?;
    for &attr in ATTRS {
        if let Some(v) = snap.attr(descendant, attr) {
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Best-effort: pull the sender display name out of a result row. Gmail
/// renders the from cell as a `<span>` with `email="..."` attribute and
/// human-readable text content.
fn scrape_row_from(snap: &crate::cdp::Snapshot, row: usize) -> Option<String> {
    let span = snap.find_descendant(row, |s, i| {
        s.is_element(i) && eq_ignore_case(s.tag(i), "span") && s.attr(i, "email").is_some()
    })?;
    let text = snap.text_content(span);
    if text.is_empty() {
        snap.attr(span, "email").map(str::to_string)
    } else {
        Some(text)
    }
}

/// Subject is the first inner span containing readable subject text.
/// Gmail wraps it in a `<span class="bog">` historically, but classes
/// rotate. Fall back to longest text run inside the row.
fn scrape_row_subject(snap: &crate::cdp::Snapshot, row: usize) -> Option<String> {
    let text = snap.text_content(row);
    if text.is_empty() {
        return None;
    }
    Some(text.chars().take(200).collect())
}

fn scrape_row_snippet(_snap: &crate::cdp::Snapshot, _row: usize) -> Option<String> {
    None
}

/// Run `from:linkedin.com` against the live Gmail UI and regex-extract
/// the user's own LinkedIn profile URL out of the matched bodies.
///
/// Stage 1 (search): drive the search input via [`search`] — clicks +
/// keyboard, no JS. Stage 2 (fetch): for each result thread, GET its
/// print-view URL through the attached CDP session (cookie-authed,
/// `cdp_fetch::fetch`). Stage 3 (extract): match `comm/in/<u>` first,
/// then `/in/<u>` — the LinkedIn notification footer always uses the
/// `comm/in/` form for the recipient's own profile.
///
/// Returns `Ok(None)` when no LinkedIn email is in the user's mailbox
/// or none of the matched bodies contains a parsable profile URL.
pub async fn find_linkedin_profile_url(account_id: &str) -> Result<Option<String>, String> {
    log::info!("[gmail][{account_id}] find_linkedin_profile_url");

    // Try a sequence of progressively broader queries. The first two
    // are Gmail's transactional LinkedIn senders — `invitations-` and
    // `notifications-` mails always carry the `comm/in/<recipient>`
    // footer URL. The last one is broad `from:linkedin.com` for
    // accounts whose only LinkedIn mail is promotional/company —
    // those usually don't carry the personal footer but we try
    // anyway as a last resort.
    const QUERIES: &[&str] = &[
        "from:invitations-noreply@linkedin.com",
        "from:notifications-noreply@linkedin.com",
        "from:linkedin.com",
    ];

    let mut messages: Vec<GmailMessage> = Vec::new();
    for q in QUERIES {
        log::info!("[gmail][{account_id}] find: trying query {:?}", q);
        let hits = search(account_id, q.to_string(), 10).await?;
        if !hits.is_empty() {
            messages = hits;
            break;
        }
    }

    log::info!(
        "[gmail][{account_id}] find_linkedin_profile_url got {} candidate threads",
        messages.len()
    );
    if messages.is_empty() {
        return Ok(None);
    }

    let (mut cdp, session) = session::attach(account_id).await?;
    let mut found: Option<String> = None;
    for msg in &messages {
        let url = format!("https://mail.google.com/mail/u/0/#inbox/{}", msg.id);
        log::debug!(
            "[gmail][{account_id}] opening thread id={} via {}",
            msg.id,
            url
        );
        if let Err(e) = cdp
            .call(
                "Page.navigate",
                serde_json::json!({ "url": url }),
                Some(&session),
            )
            .await
        {
            log::warn!(
                "[gmail][{account_id}] thread navigate failed id={} err={}",
                msg.id,
                e
            );
            continue;
        }
        // Wait for the thread view to render and re-snapshot up to 6
        // times. Gmail's SPA finishes lazily after the hash change; the
        // body iframe takes a beat. The Snapshot is now iframe-aware so
        // a successful render exposes the email body anchors here.
        for _ in 0..6 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let snap = match crate::cdp::Snapshot::capture(&mut cdp, &session).await {
                Ok(s) => s,
                Err(e) => {
                    log::warn!(
                        "[gmail][{account_id}] thread snapshot failed id={} err={}",
                        msg.id, e
                    );
                    continue;
                }
            };
            if let Some(u) = scrape_linkedin_url_from_dom(&snap) {
                found = Some(u);
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }
    session::detach(&mut cdp, &session).await;
    log::info!(
        "[gmail][{account_id}] find_linkedin_profile_url done found={:?}",
        found
    );
    Ok(found)
}

/// Walk the rendered Gmail thread-view DOM for any `<a>` whose `href`
/// points at a LinkedIn profile (`linkedin.com/comm/in/<u>` or
/// `linkedin.com/in/<u>`), and return the canonical profile URL.
///
/// Gmail rewrites every external link inside email bodies to go
/// through `https://www.google.com/url?q=<destination>&...`, so we
/// also scan that wrapper form.
fn scrape_linkedin_url_from_dom(snap: &crate::cdp::Snapshot) -> Option<String> {
    let anchors = snap.find_all(|s, i| {
        eq_ignore_case(s.tag(i), "a") && s.attr(i, "href").is_some()
    });
    for idx in anchors {
        let href = snap.attr(idx, "href").unwrap_or("");
        // Direct LinkedIn link.
        if let Some(u) = extract_linkedin_url(href) {
            return Some(u);
        }
        // Google click-tracker `https://www.google.com/url?q=<dest>&...`.
        if let Some(q_start) = href.find("?q=") {
            let after = &href[q_start + 3..];
            let dest_end = after.find('&').unwrap_or(after.len());
            let dest_raw = &after[..dest_end];
            // Lightweight URL-decode: only `%2F`, `%3A`, etc. show up
            // in the q= param; we just need the LinkedIn substring to
            // resolve.
            let dest = dest_raw.replace("%2F", "/").replace("%3A", ":");
            if let Some(u) = extract_linkedin_url(&dest) {
                return Some(u);
            }
        }
    }
    None
}

/// Pull a LinkedIn profile URL out of an email body. Tries the
/// `comm/in/<u>` notification-footer form first (always the recipient's
/// own profile in linkedin.com mails), then the canonical `/in/<u>`
/// form as a fallback.
///
/// Username charset matches LinkedIn's vanity-URL spec (alphanumerics,
/// `-`, `_`).
fn extract_linkedin_url(body: &str) -> Option<String> {
    if let Some(u) = scan_linkedin_pattern(body, "linkedin.com/comm/in/") {
        return Some(format!("https://www.linkedin.com/in/{u}"));
    }
    if let Some(u) = scan_linkedin_pattern(body, "linkedin.com/in/") {
        return Some(format!("https://www.linkedin.com/in/{u}"));
    }
    None
}

fn scan_linkedin_pattern(body: &str, anchor: &str) -> Option<String> {
    let mut search_from = 0usize;
    while let Some(rel) = body[search_from..].find(anchor) {
        let start = search_from + rel + anchor.len();
        let tail = &body[start..];
        let end = tail
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
            .unwrap_or(tail.len());
        if end > 0 {
            return Some(tail[..end].to_string());
        }
        search_from = start;
    }
    None
}

/// Extract `<digits>` from a string that contains `permthid=thread-f:<digits>`
/// somewhere. Returns `None` when the pattern isn't present.
fn extract_permthid(s: &str) -> Option<String> {
    let needle = "permthid=thread-f:";
    let start = s.find(needle)? + needle.len();
    let tail = &s[start..];
    let end = tail
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(tail.len());
    if end == 0 {
        None
    } else {
        Some(tail[..end].to_string())
    }
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
/// * Hex thread ids via `th=<hex>` — what the inbox UI uses internally
///   and what `<tr class="zA">` rows surface via
///   `data-legacy-thread-id` / `data-thread-id`.
/// * Decimal `thread-f:` ids via `permthid=thread-f:<dec>&permmsgid=msg-f:<dec>`
///   — this is what the Atom feed gives us.
///
/// We auto-detect: pure-digit ids go to the decimal form (compatible
/// with `list_messages` callers), anything containing hex chars uses
/// the `th=<hex>` form (what `search` returns).
fn print_view_url(message_id: &str) -> String {
    let escaped = url_path_escape(message_id);
    if message_id.chars().all(|c| c.is_ascii_digit()) {
        format!(
            "https://mail.google.com/mail/u/0/?ui=2&view=pt&search=all\
             &permthid=thread-f:{escaped}&permmsgid=msg-f:{escaped}"
        )
    } else {
        format!(
            "https://mail.google.com/mail/u/0/?ui=2&view=pt&search=all&th={escaped}"
        )
    }
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
    fn extract_linkedin_url_prefers_comm_in() {
        let body = "<p>See <a href=\"https://www.linkedin.com/comm/in/jane-doe-123/\">profile</a></p>";
        assert_eq!(
            extract_linkedin_url(body),
            Some("https://www.linkedin.com/in/jane-doe-123".into())
        );
    }

    #[test]
    fn extract_linkedin_url_handles_regional_subdomains() {
        // LinkedIn rotates the regional subdomain in notification mail
        // (ae.linkedin.com, in.linkedin.com, fr.linkedin.com, …). The
        // username extractor needs to anchor on `linkedin.com/comm/in/`
        // as a substring, NOT match a fixed origin. Trailing query
        // params like `?sdfsdf` (LinkedIn click trackers) must also be
        // stripped from the username.
        let body = "https://ae.linkedin.com/comm/in/senamakel?sdfsdf";
        assert_eq!(
            extract_linkedin_url(body),
            Some("https://www.linkedin.com/in/senamakel".into())
        );

        let body2 = "Visit https://in.linkedin.com/in/john_smith?utm_source=email cordially.";
        assert_eq!(
            extract_linkedin_url(body2),
            Some("https://www.linkedin.com/in/john_smith".into())
        );
    }

    #[test]
    fn extract_linkedin_url_falls_back_to_in() {
        let body = "Visit linkedin.com/in/john_smith_42 today.";
        assert_eq!(
            extract_linkedin_url(body),
            Some("https://www.linkedin.com/in/john_smith_42".into())
        );
    }

    #[test]
    fn extract_linkedin_url_returns_none_without_match() {
        assert_eq!(extract_linkedin_url("nothing relevant here"), None);
    }

    #[test]
    fn extract_permthid_pulls_decimal_id() {
        assert_eq!(
            extract_permthid("?ui=2&view=pt&permthid=thread-f:1234567890&permmsgid=msg-f:0"),
            Some("1234567890".into())
        );
        assert_eq!(
            extract_permthid("https://example/#search/foo?permthid=thread-f:42"),
            Some("42".into())
        );
        assert_eq!(extract_permthid("no match here"), None);
        assert_eq!(extract_permthid("permthid=thread-f:"), None);
    }

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
