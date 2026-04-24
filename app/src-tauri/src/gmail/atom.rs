//! Gmail Atom feed parser.
//!
//! Gmail exposes a stable Atom feed at
//! `https://mail.google.com/mail/u/0/feed/atom[/<label>]` that returns
//! the 20 most recent **unread** messages as XML. Example entry:
//!
//! ```xml
//! <entry>
//!   <title>Re: Project sync</title>
//!   <summary>Quick status update before Friday…</summary>
//!   <link href="https://mail.google.com/mail/u/0/?…&message_id=181a3b4c…"/>
//!   <modified>2026-04-23T18:12:00Z</modified>
//!   <issued>2026-04-23T18:12:00Z</issued>
//!   <id>tag:gmail.google.com,2004:181a3b4c…</id>
//!   <author><name>Alice</name><email>alice@example.com</email></author>
//! </entry>
//! ```
//!
//! This parser is intentionally small and tolerant: tag attributes
//! beyond what we read are ignored, unknown entries are skipped rather
//! than failing the whole parse, and the `<summary>` / `<title>` text
//! is XML-entity-decoded.

use crate::gmail::types::GmailMessage;

/// Parse the Atom body into `GmailMessage`s. Returns the unread entries
/// in document order. Never panics on malformed input — returns an
/// empty Vec if nothing matches.
pub fn parse(body: &str) -> Vec<GmailMessage> {
    let mut out = Vec::new();
    for entry_xml in iter_tag(body, "entry") {
        let title = find_tag(entry_xml, "title").as_deref().map(decode_entities);
        let summary = find_tag(entry_xml, "summary")
            .as_deref()
            .map(decode_entities);
        let modified = find_tag(entry_xml, "modified");
        let issued = find_tag(entry_xml, "issued");
        let id_raw = find_tag(entry_xml, "id").unwrap_or_default();
        let id = strip_id_prefix(&id_raw);
        let link_href = find_link_href(entry_xml);
        let author_xml = find_tag(entry_xml, "author").unwrap_or_default();
        let from_name = find_tag(&author_xml, "name")
            .as_deref()
            .map(decode_entities);
        let from_email = find_tag(&author_xml, "email")
            .as_deref()
            .map(decode_entities);
        let from: Option<String> = match (from_name.as_deref(), from_email.as_deref()) {
            (Some(n), Some(e)) => Some(format!("{n} <{e}>")),
            (None, Some(e)) => Some(e.to_string()),
            (Some(n), None) => Some(n.to_string()),
            _ => None,
        };
        if id.is_empty() && link_href.is_empty() && title.is_none() {
            // Completely empty entry — skip rather than surface garbage.
            continue;
        }
        let date_ms = modified
            .as_deref()
            .or(issued.as_deref())
            .and_then(parse_iso8601_ms);
        // All atom entries are unread by construction — the feed only
        // returns unread messages.
        out.push(GmailMessage {
            id,
            thread_id: None,
            from,
            to: Vec::new(),
            cc: Vec::new(),
            subject: title,
            snippet: summary,
            body: None,
            date_ms,
            labels: vec!["INBOX".into()],
            unread: true,
        });
    }
    out
}

/// Iterator over the inner text of every `<TAG>…</TAG>` slice in `body`.
/// Non-recursive and best-effort — repeated/nested tags with the same
/// name would each yield their own slice.
fn iter_tag<'a>(body: &'a str, tag: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut cursor = 0usize;
    std::iter::from_fn(move || {
        while cursor < body.len() {
            let slice = &body[cursor..];
            let start = slice.find(&open)?;
            let after_open = cursor + start + open.len();
            // Skip the rest of the opening tag up to '>' (attributes etc).
            let rest = body.get(after_open..)?;
            let gt_rel = rest.find('>')?;
            let content_start = after_open + gt_rel + 1;
            let tail = body.get(content_start..)?;
            let close_rel = tail.find(&close)?;
            let content_end = content_start + close_rel;
            cursor = content_end + close.len();
            return Some(&body[content_start..content_end]);
        }
        None
    })
}

/// Find the first `<TAG>…</TAG>` and return its inner text. Returns
/// `None` if the tag is absent.
fn find_tag(body: &str, tag: &str) -> Option<String> {
    iter_tag(body, tag).next().map(|s| s.trim().to_string())
}

/// `<link href="…"/>` — self-closing, attribute-carried. Atom links
/// don't have inner text, so we special-case the parse.
fn find_link_href(body: &str) -> String {
    // Simple attribute scan — good enough for the fixed shape Gmail
    // emits. We don't need generic HTML attribute parsing.
    let needle = "<link";
    if let Some(start) = body.find(needle) {
        let rest = &body[start + needle.len()..];
        if let Some(end) = rest.find('>') {
            let attrs = &rest[..end];
            if let Some(h) = find_attr(attrs, "href") {
                return h;
            }
        }
    }
    String::new()
}

fn find_attr(attrs: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let pos = attrs.find(&key)?;
    let after = &attrs[pos + key.len()..];
    let end = after.find('"')?;
    Some(decode_entities(&after[..end]))
}

/// Strip the `tag:gmail.google.com,2004:` prefix Gmail uses, leaving
/// just the numeric message id. Falls back to the raw string when the
/// prefix is missing so we never drop identifiers silently.
fn strip_id_prefix(raw: &str) -> String {
    const PREFIX: &str = "tag:gmail.google.com,2004:";
    raw.trim()
        .strip_prefix(PREFIX)
        .unwrap_or_else(|| raw.trim())
        .to_string()
}

/// Small XML entity decoder — covers the five predefined named
/// entities plus `&#NN;` / `&#xHH;` numeric escapes. Walks chars
/// (not bytes) so multi-byte UTF-8 passes through untouched.
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
            // Unterminated `&` — keep the rest verbatim.
            out.push_str(after);
            return out;
        }
    }
    out.push_str(rest);
    out
}

/// Minimal ISO 8601 → unix millis parser for `2026-04-23T18:12:00Z`
/// shapes Gmail emits. Accepts both `Z` and explicit `+00:00` suffix.
/// Returns `None` on anything exotic — callers fall back to `None`
/// date rather than an invented value.
fn parse_iso8601_ms(s: &str) -> Option<i64> {
    // Very narrow: YYYY-MM-DDTHH:MM:SSZ (optionally with fractional
    // seconds). No TZ offset parsing beyond 'Z'.
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.len() < 20 {
        return None;
    }
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let mo: u32 = s.get(5..7)?.parse().ok()?;
    let d: u32 = s.get(8..10)?.parse().ok()?;
    let h: u32 = s.get(11..13)?.parse().ok()?;
    let mi: u32 = s.get(14..16)?.parse().ok()?;
    let se: u32 = s.get(17..19)?.parse().ok()?;
    // Days since unix epoch using a (reasonably) small leap-year aware
    // calc. We don't pull `chrono` just for this conversion because
    // the atom feed ISO-8601 is always UTC.
    let days = days_from_civil(y, mo, d);
    let secs = days * 86_400 + h as i64 * 3600 + mi as i64 * 60 + se as i64;
    Some(secs.saturating_mul(1000))
}

/// Howard Hinnant's civil-from-days, adapted. Correct for all dates
/// Gmail emits (and anything up to year 10000).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = (y - era * 400) as i64; // [0, 399]
    let m = m as i64;
    let d = d as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://purl.org/atom/ns#">
  <title>Gmail - Inbox for user@example.com</title>
  <tagline>New messages in your Gmail inbox</tagline>
  <fullcount>2</fullcount>
  <link rel="alternate" href="https://mail.google.com/mail" type="text/html"/>
  <modified>2026-04-23T18:30:00Z</modified>
  <entry>
    <title>Re: Project &amp; plan</title>
    <summary>Quick status update &lt;EOM&gt;</summary>
    <link rel="alternate" href="https://mail.google.com/mail/u/0/?t=1" type="text/html"/>
    <modified>2026-04-23T18:12:00Z</modified>
    <issued>2026-04-23T18:12:00Z</issued>
    <id>tag:gmail.google.com,2004:181a3b4c001</id>
    <author><name>Alice</name><email>alice@example.com</email></author>
  </entry>
  <entry>
    <title>Receipt from Acme</title>
    <summary>Order #42 confirmed</summary>
    <link rel="alternate" href="https://mail.google.com/mail/u/0/?t=2" type="text/html"/>
    <modified>2026-04-23T12:00:00Z</modified>
    <issued>2026-04-23T12:00:00Z</issued>
    <id>tag:gmail.google.com,2004:181a3b4c002</id>
    <author><name>Acme</name><email>no-reply@acme.example</email></author>
  </entry>
</feed>"#;

    #[test]
    fn parses_two_entries_with_subject_from_snippet() {
        let msgs = parse(SAMPLE);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].id, "181a3b4c001");
        assert_eq!(msgs[0].subject.as_deref(), Some("Re: Project & plan"));
        assert_eq!(
            msgs[0].snippet.as_deref(),
            Some("Quick status update <EOM>")
        );
        assert_eq!(msgs[0].from.as_deref(), Some("Alice <alice@example.com>"));
        assert!(msgs[0].unread);
        assert_eq!(msgs[0].labels, vec!["INBOX".to_string()]);
        assert!(msgs[0].date_ms.is_some());
    }

    #[test]
    fn decodes_xml_entities_in_subject_and_body() {
        let msgs = parse(SAMPLE);
        assert!(msgs[0].subject.as_deref().unwrap().contains(" & "));
        assert!(msgs[0].snippet.as_deref().unwrap().contains("<"));
    }

    #[test]
    fn iso_8601_to_ms_round_trips_known_epoch() {
        // 2026-04-23T18:12:00Z
        let ms = parse_iso8601_ms("2026-04-23T18:12:00Z").unwrap();
        // Sanity: positive and within a plausible 2026 range.
        assert!(ms > 1_775_000_000_000);
        assert!(ms < 1_800_000_000_000);
    }

    #[test]
    fn empty_body_returns_empty_vec() {
        assert!(parse("").is_empty());
        assert!(parse("<feed></feed>").is_empty());
    }

    #[test]
    fn decode_entities_handles_numeric_and_utf8() {
        assert_eq!(decode_entities("it&#39;s"), "it's");
        assert_eq!(decode_entities("&#x2022; bullet"), "• bullet");
        // Multi-byte UTF-8 must pass through unchanged — this was the
        // bug that mangled zero-width-joiner bytes in the first cut.
        assert_eq!(decode_entities("café"), "café");
        assert_eq!(decode_entities("a&amp;b &lt;x&gt;"), "a&b <x>");
    }

    #[test]
    fn missing_fields_in_entry_still_yields_message() {
        let body = r#"<feed><entry><id>tag:gmail.google.com,2004:abc</id></entry></feed>"#;
        let msgs = parse(body);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].id, "abc");
        assert!(msgs[0].subject.is_none());
        assert!(msgs[0].from.is_none());
    }
}
