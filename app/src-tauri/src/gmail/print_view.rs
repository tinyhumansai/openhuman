//! Gmail print-view HTML parser.
//!
//! The print view at `mail.google.com/mail/u/0/?ui=2&view=pt&th=<id>`
//! returns a simple, mostly-table-based HTML page with stable markers:
//!
//! ```html
//! <title>Gmail - Subject line</title>
//! …
//! <div id=":…" class="ii gt">  (body container)
//! ```
//!
//! The specific structure has varied over the years but these anchors
//! are extremely sticky. We parse defensively — missing fields become
//! `None` rather than fail the whole op.

use crate::gmail::types::GmailMessage;

/// Parse one rendered print-view HTML page into a `GmailMessage`.
/// Returns `None` only when the response didn't look like a Gmail
/// print view at all (e.g. login redirect HTML).
///
/// Implementation note: Gmail's real print view is a flat sequence of
/// blocks — subject (via `<title>`), sender + date, a `To: …` line,
/// then the body — without the `<th>`/`<td>` table shape older scrapers
/// relied on. We convert the page to plaintext (strip tags, collapse
/// whitespace, insert line breaks at block-level boundaries) and then
/// pick fields out of the resulting line stream. This is more stable
/// across Gmail UI churn than anchoring on specific tag shapes.
pub fn parse(message_id: &str, html: &str) -> Option<GmailMessage> {
    if !looks_like_print_view(html) {
        return None;
    }

    let subject = extract_subject(html);
    let lines = html_to_text_lines(html);

    // Walk the flattened lines looking for sender/date/to/body anchors.
    // The typical Gmail print-view shape (post tag-strip) is:
    //   {Subject line — same as title}
    //   {Account-owner email — "Steven Enamakel <me@…>"}
    //   {Actual sender — "Trip.com <…>"}
    //   {Date: "Thu, Apr 23, 2026 at 9:01 PM"}
    //   {To: me@…}
    //   {blank}
    //   {body …}
    //
    // We can't tell account-owner from sender by their line shape alone,
    // so collect every email-like candidate in order and pick the LAST
    // one before the `To:` line as the sender. That matches both the
    // 2-header and 1-header layouts Gmail has shipped.
    let mut date_ms: Option<i64> = None;
    let mut to: Vec<String> = Vec::new();
    let mut cc: Vec<String> = Vec::new();
    let mut from_candidates: Vec<String> = Vec::new();
    let mut to_line_idx: Option<usize> = None;
    let mut body_start_idx: Option<usize> = None;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Skip the subject echo that Gmail repeats below the <title>.
        if subject.as_deref() == Some(trimmed) {
            continue;
        }
        if let Some(rest) = ci_strip_prefix(trimmed, "to:") {
            to = split_addresses(Some(rest.trim()));
            to_line_idx = Some(i);
            continue;
        }
        if let Some(rest) = ci_strip_prefix(trimmed, "cc:") {
            cc = split_addresses(Some(rest.trim()));
            continue;
        }
        // Collect ALL email-like lines pre-To. We'll pick the last one
        // as the sender below.
        if to_line_idx.is_none() && looks_like_sender(trimmed) {
            from_candidates.push(trimmed.to_string());
            continue;
        }
        // Date can appear before or after "To:" — accept anywhere.
        if date_ms.is_none() {
            if let Some(ms) = try_parse_gmail_date(trimmed) {
                date_ms = Some(ms);
                continue;
            }
        }
        // Once we've seen To:, the next non-meta line opens the body.
        if body_start_idx.is_none() && to_line_idx.is_some() && !looks_like_meta(trimmed) {
            body_start_idx = Some(i);
            break;
        }
    }

    // Pick the last email-like line before "To:" as the sender — that's
    // the actual sender in both Gmail print-view layouts. If there were
    // zero candidates, fall through with `from = None`.
    let from: Option<String> = from_candidates.into_iter().last();

    let body: Option<String> = body_start_idx.map(|start| {
        let joined = lines[start..]
            .iter()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        joined.trim().to_string()
    });
    let snippet = body.as_deref().map(derive_snippet);

    Some(GmailMessage {
        id: message_id.to_string(),
        thread_id: Some(message_id.to_string()),
        from,
        to,
        cc,
        subject,
        snippet,
        body,
        date_ms,
        labels: Vec::new(),
        unread: false,
    })
}

/// Heuristics. The sender line is typically `Name <email@dom.com>` or
/// a bare `email@dom.com`. We require at least one `@` to avoid
/// matching random noise.
fn looks_like_sender(s: &str) -> bool {
    s.contains('@') && s.len() < 256
}

/// Lines like `To:`, `From:`, `Cc:`, `Subject:` that aren't body text.
fn looks_like_meta(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    ["to:", "cc:", "bcc:", "from:", "subject:", "date:"]
        .iter()
        .any(|p| lower.starts_with(p))
}

fn ci_strip_prefix<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let lower_prefix = prefix.to_ascii_lowercase();
    if s.len() < prefix.len() {
        return None;
    }
    let head = s.get(..prefix.len())?;
    if head.to_ascii_lowercase() == lower_prefix {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

/// Convert HTML to a stream of plaintext lines.
///   * Drops everything inside `<script>` / `<style>` tags.
///   * Inserts `\n` at `<br>`, `<p>`, `<div>`, `<tr>`, `<li>`, `<h*>`
///     tag boundaries.
///   * Decodes HTML entities.
fn html_to_text_lines(html: &str) -> Vec<String> {
    let cleaned = strip_script_style(html);
    let with_breaks = insert_block_breaks(&cleaned);
    let stripped = strip_tags(&with_breaks);
    let decoded = decode_entities(&stripped);
    decoded.lines().map(str::to_string).collect()
}

fn strip_script_style(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    loop {
        let lower = rest.to_ascii_lowercase();
        let s_pos = lower.find("<script").map(|p| (p, "</script>"));
        let y_pos = lower.find("<style").map(|p| (p, "</style>"));
        let next = [s_pos, y_pos]
            .iter()
            .filter_map(|x| x.clone())
            .min_by_key(|(p, _)| *p);
        match next {
            Some((start, closer)) => {
                out.push_str(&rest[..start]);
                let lower_tail = rest[start..].to_ascii_lowercase();
                if let Some(end) = lower_tail.find(closer) {
                    let skip_to = start + end + closer.len();
                    rest = &rest[skip_to..];
                } else {
                    // Unterminated — bail out, take the rest as-is.
                    return out;
                }
            }
            None => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

fn insert_block_breaks(html: &str) -> String {
    // Cheap approximation: replace each known block-start / block-end
    // tag with a newline sentinel. Case-insensitive. Gmail's print
    // view lays headers out in `<td>`/`<span>`/`<b>` cells without
    // explicit `<br>` between them, so we break on the closers of
    // those too to get one field per line after the strip.
    let mut s = html.to_string();
    for tag in [
        "<br", "</p", "</div", "</tr", "</li", "</h1", "</h2", "</h3", "</h4", "</td", "</span",
        "</b>", "<p ", "<p>", "<div ", "<div>", "<tr ", "<tr>", "<li ", "<li>", "<td ", "<td>",
    ] {
        let lower = s.to_ascii_lowercase();
        if !lower.contains(tag) {
            continue;
        }
        // We walk the lowercase index but rewrite in the original `s`.
        let mut out = String::with_capacity(s.len() + 16);
        let mut i = 0;
        while i < s.len() {
            let lower_slice = &lower[i..];
            if let Some(pos) = lower_slice.find(tag) {
                out.push_str(&s[i..i + pos]);
                out.push('\n');
                i += pos;
                // Find the tag close '>' and include it.
                if let Some(end) = s[i..].find('>') {
                    out.push_str(&s[i..i + end + 1]);
                    i += end + 1;
                } else {
                    out.push_str(&s[i..]);
                    break;
                }
            } else {
                out.push_str(&s[i..]);
                break;
            }
        }
        s = out;
    }
    s
}

/// Try a few loose date shapes Gmail emits in the print view:
///   * `Thu, Apr 23, 2026 at 9:01 PM`
///   * `Thu, Apr 23, 2026, 9:01 PM`
///   * RFC 2822 `Thu, 23 Apr 2026 18:12:00 -0700`
fn try_parse_gmail_date(s: &str) -> Option<i64> {
    parse_rfc2822_ms(s).or_else(|| parse_human_date_ms(s))
}

/// `Thu, Apr 23, 2026 at 9:01 PM` — the form the Gmail print view emits.
fn parse_human_date_ms(s: &str) -> Option<i64> {
    let s = s.trim().trim_end_matches('.');
    // Normalize separators: drop commas and the word "at".
    let norm: String = s
        .replace(',', " ")
        .replace(" at ", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut it = norm.split_whitespace();
    let _dow = it.next()?; // Thu
    let mon = month_from_str(it.next()?)?;
    let day: u32 = it.next()?.parse().ok()?;
    let year: i64 = it.next()?.parse().ok()?;
    let time_tok = it.next()?;
    let ampm = it.next().unwrap_or("");
    let mut th = time_tok.split(':');
    let mut hh: u32 = th.next()?.parse().ok()?;
    let mm: u32 = th.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    match ampm.to_ascii_uppercase().as_str() {
        "PM" if hh < 12 => hh += 12,
        "AM" if hh == 12 => hh = 0,
        _ => {}
    }
    let days = days_from_civil(year, mon, day);
    // Human date has no TZ — treat as local-naive (we don't know the
    // mailbox's TZ), so use UTC. Caller sees the absolute millis.
    let secs = days * 86_400 + hh as i64 * 3600 + mm as i64 * 60;
    Some(secs.saturating_mul(1000))
}

fn looks_like_print_view(html: &str) -> bool {
    // Gmail print-view pages always contain these two markers:
    //  - `<title>Gmail -` — the browser tab title
    //  - a `From:`-labelled header row
    // Login / error pages contain neither.
    html.contains("<title>Gmail -") || html.to_ascii_lowercase().contains(">from:</")
}

/// `<title>Gmail - {subject}</title>` → the subject portion.
fn extract_subject(html: &str) -> Option<String> {
    let start = html.find("<title>")? + "<title>".len();
    let end = html[start..].find("</title>")? + start;
    let raw = decode_entities(&html[start..end]).trim().to_string();
    // Strip the leading "Gmail - " prefix if present.
    Some(
        raw.strip_prefix("Gmail - ")
            .map(str::to_string)
            .unwrap_or(raw),
    )
}

fn derive_snippet(body: &str) -> String {
    // Collapse whitespace and cap at 180 chars to match Gmail UI-ish
    // snippet length.
    let collapsed: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() > 180 {
        format!(
            "{}…",
            &collapsed[..collapsed
                .char_indices()
                .nth(180)
                .map(|(i, _)| i)
                .unwrap_or(180)]
        )
    } else {
        collapsed
    }
}

fn split_addresses(raw: Option<&str>) -> Vec<String> {
    match raw {
        None => Vec::new(),
        Some(s) => s
            .split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect(),
    }
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Small HTML entity decoder — named + numeric. Char-aware so
/// multi-byte UTF-8 passes through unchanged. Same shape as the one
/// in `atom.rs` but also handles `&nbsp;`.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp..];
        if let Some(semi_rel) = after.find(';') {
            let tok = &after[1..semi_rel];
            let replaced = match tok {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                t if t.starts_with("#x") || t.starts_with("#X") => u32::from_str_radix(&t[2..], 16)
                    .ok()
                    .and_then(char::from_u32),
                t if t.starts_with('#') => t[1..].parse::<u32>().ok().and_then(char::from_u32),
                _ => None,
            };
            match replaced {
                Some(ch) => out.push(ch),
                None => out.push_str(&after[..=semi_rel]),
            }
            rest = &after[semi_rel + 1..];
        } else {
            out.push_str(after);
            return out;
        }
    }
    out.push_str(rest);
    out
}

/// Very narrow RFC-2822 date → unix millis. Gmail emits dates like
/// `Thu, 23 Apr 2026 18:12:00 -0700`. We handle that shape and return
/// `None` for anything exotic — callers surface a `None` date rather
/// than an invented one.
fn parse_rfc2822_ms(s: &str) -> Option<i64> {
    // Cheap heuristic parse. Good enough for the shape Gmail emits.
    let mut parts = s.split_whitespace();
    let _dow = parts.next()?; // "Thu,"
    let day: u32 = parts.next()?.parse().ok()?;
    let mon = month_from_str(parts.next()?)?;
    let year: i64 = parts.next()?.parse().ok()?;
    let hms = parts.next()?;
    let mut h_iter = hms.split(':');
    let hh: u32 = h_iter.next()?.parse().ok()?;
    let mm: u32 = h_iter.next()?.parse().ok()?;
    let ss: u32 = h_iter.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let tz = parts.next().unwrap_or("+0000");
    let tz_offset_s = parse_tz_offset_secs(tz).unwrap_or(0);
    let days = days_from_civil(year, mon, day);
    let secs = days * 86_400 + hh as i64 * 3600 + mm as i64 * 60 + ss as i64 - tz_offset_s;
    Some(secs.saturating_mul(1000))
}

fn month_from_str(m: &str) -> Option<u32> {
    Some(match m {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    })
}

fn parse_tz_offset_secs(tz: &str) -> Option<i64> {
    if tz == "GMT" || tz == "UTC" || tz == "Z" {
        return Some(0);
    }
    let (sign, digits) = match tz.as_bytes().first()? {
        b'+' => (1, &tz[1..]),
        b'-' => (-1, &tz[1..]),
        _ => return None,
    };
    if digits.len() != 4 {
        return None;
    }
    let h: i64 = digits.get(0..2)?.parse().ok()?;
    let m: i64 = digits.get(2..4)?.parse().ok()?;
    Some(sign * (h * 3600 + m * 60))
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = m as i64;
    let d = d as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real Gmail print-view shape (anonymised). Flatter than the older
    /// `<th>`/`<td>` table form — sender-and-email inline, date on its
    /// own line in a human-readable form, `To:` labelled line, then body.
    const SAMPLE_HTML: &str = r#"<html><head><title>Gmail - Re: Project &amp; plan</title></head>
<body>
<div>Gmail - Re: Project &amp; plan</div>
<div>&quot;Alice&quot; &lt;alice@example.com&gt;</div>
<div>Thu, Apr 23, 2026 at 6:12 PM</div>
<div>To: me@example.com, &quot;Bob&quot; &lt;bob@example.com&gt;</div>
<div>Hello team — quick status update before Friday&#39;s demo.<br>Will send slides later.</div>
</body></html>"#;

    #[test]
    fn parses_subject_from_to_and_body() {
        let m = parse("181a3b4c001", SAMPLE_HTML).unwrap();
        assert_eq!(m.id, "181a3b4c001");
        assert_eq!(m.subject.as_deref(), Some("Re: Project & plan"));
        assert!(
            m.from.as_deref().unwrap().contains("alice@example.com"),
            "unexpected from: {:?}",
            m.from
        );
        assert_eq!(m.to.len(), 2);
        assert!(
            m.to.iter().any(|t| t.contains("bob@example.com")),
            "to missing bob: {:?}",
            m.to
        );
        assert!(
            m.body.as_deref().unwrap().contains("Hello team"),
            "body: {:?}",
            m.body
        );
        assert!(m.date_ms.is_some(), "date_ms: {:?}", m.date_ms);
    }

    #[test]
    fn returns_none_on_non_print_view_html() {
        let html = "<html><body>Please sign in</body></html>";
        assert!(parse("x", html).is_none());
    }

    #[test]
    fn snippet_is_short_and_whitespace_collapsed() {
        let m = parse("id", SAMPLE_HTML).unwrap();
        let snippet = m.snippet.unwrap();
        assert!(snippet.len() <= 181);
        assert!(!snippet.contains('\n'));
    }

    /// Gmail's real print view shows an account-owner line BEFORE the
    /// actual sender. Our `from_candidates`/pick-last rule must favour
    /// the sender (second email-like line pre-`To:`), not the account.
    #[test]
    fn picks_sender_not_account_owner_from_two_header_layout() {
        let html = r#"<html><head><title>Gmail - Receipt</title></head>
<body>
<span>Gmail - Receipt</span>
<span>Steven Enamakel &lt;me@example.com&gt;</span>
<span>Trip.com &lt;noreply@trip.com&gt;</span>
<span>Thu, Apr 23, 2026 at 9:01 PM</span>
<span>To: me@example.com</span>
<div>Your new Trip Coins balance is 69.</div>
</body></html>"#;
        let m = parse("abc", html).unwrap();
        assert_eq!(
            m.from.as_deref(),
            Some("Trip.com <noreply@trip.com>"),
            "from: {:?}",
            m.from
        );
        assert_eq!(m.to, vec!["me@example.com".to_string()]);
        assert!(m.date_ms.is_some(), "expected human date parse");
        assert!(m.body.as_deref().unwrap().contains("Trip Coins"));
    }

    #[test]
    fn rfc2822_date_parses_with_non_utc_offset() {
        let ms_utc = parse_rfc2822_ms("Thu, 23 Apr 2026 18:12:00 +0000").unwrap();
        let ms_pdt = parse_rfc2822_ms("Thu, 23 Apr 2026 11:12:00 -0700").unwrap();
        assert_eq!(ms_utc, ms_pdt);
    }
}
