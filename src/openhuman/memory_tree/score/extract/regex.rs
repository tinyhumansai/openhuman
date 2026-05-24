//! Deterministic mechanical-entity extraction via regex.
//!
//! Catches the shapes regex handles cleanly and that are genuinely useful
//! as cross-platform identity anchors (email appearing in Slack + Gmail =
//! same person):
//!
//! - **Email** — RFC-ish pattern, boundary-guarded
//! - **URL** — `http(s)://…` up to whitespace or trailing punctuation
//! - **Handle** — `@alice`, `@alice.bsky.social`, or `alice#1234`
//! - **Hashtag** — `#launch-q2`
//!
//! Every match has `score = 1.0` (regex is deterministic). Spans are
//! char-offsets (not bytes) for UTF-8 safety.

use once_cell::sync::Lazy;
use regex::Regex;

use super::types::{EntityKind, ExtractedEntities, ExtractedEntity, ExtractedTopic};

// ── Compiled regexes (once per process) ──────────────────────────────────

static RE_EMAIL: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b[a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{2,}\b").unwrap());

static RE_URL: Lazy<Regex> = Lazy::new(|| {
    // up-to trailing punctuation; avoids catastrophic backtracking
    Regex::new(r"https?://[^\s<>\]\[()]+[^\s<>\]\[()\.\,;:\!\?]").unwrap()
});

static RE_HANDLE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?:^|[\s(])@([A-Za-z0-9_][A-Za-z0-9_.\-]{1,})").unwrap());

static RE_DISCRIM: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b([A-Za-z0-9_.\-]{2,32})#\d{4}\b").unwrap());

static RE_HASHTAG: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?:^|[\s(])#([A-Za-z][A-Za-z0-9_\-]{1,})").unwrap());

/// Extract all mechanical entities from `text`.
pub fn extract(text: &str) -> ExtractedEntities {
    let mut entities: Vec<ExtractedEntity> = Vec::new();
    let mut topics: Vec<ExtractedTopic> = Vec::new();

    for m in RE_EMAIL.find_iter(text) {
        entities.push(to_entity(text, m.start(), m.end(), EntityKind::Email));
    }
    for m in RE_URL.find_iter(text) {
        entities.push(to_entity(text, m.start(), m.end(), EntityKind::Url));
    }
    for cap in RE_HANDLE.captures_iter(text) {
        if let Some(m) = cap.get(1) {
            entities.push(to_entity(text, m.start(), m.end(), EntityKind::Handle));
        }
    }
    for cap in RE_DISCRIM.captures_iter(text) {
        if let Some(m) = cap.get(0) {
            entities.push(to_entity(text, m.start(), m.end(), EntityKind::Handle));
        }
    }
    for cap in RE_HASHTAG.captures_iter(text) {
        if let Some(m) = cap.get(1) {
            entities.push(to_entity(text, m.start(), m.end(), EntityKind::Hashtag));
            topics.push(ExtractedTopic {
                label: text[m.start()..m.end()].to_lowercase(),
                score: 1.0,
            });
        }
    }

    ExtractedEntities {
        entities,
        topics,
        // Regex extractor never produces an LLM importance signal.
        llm_importance: None,
        llm_importance_reason: None,
    }
}

fn to_entity(text: &str, start: usize, end: usize, kind: EntityKind) -> ExtractedEntity {
    ExtractedEntity {
        kind,
        text: text[start..end].to_string(),
        span_start: char_index(text, start),
        span_end: char_index(text, end),
        score: 1.0,
    }
}

fn char_index(s: &str, byte_idx: usize) -> u32 {
    let byte_idx = byte_idx.min(s.len());
    s[..byte_idx].chars().count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(e: &ExtractedEntities) -> Vec<EntityKind> {
        let mut k: Vec<_> = e.entities.iter().map(|x| x.kind).collect();
        k.sort_by_key(|k| *k as u8);
        k
    }

    #[test]
    fn email_basic() {
        let o = extract("contact alice@example.com please");
        assert_eq!(o.entities.len(), 1);
        assert_eq!(o.entities[0].kind, EntityKind::Email);
        assert_eq!(o.entities[0].text, "alice@example.com");
    }

    #[test]
    fn url_stops_at_trailing_punct() {
        let o = extract("see https://example.com/x?y=1 now.");
        let urls: Vec<_> = o
            .entities
            .iter()
            .filter(|e| e.kind == EntityKind::Url)
            .collect();
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].text, "https://example.com/x?y=1");
    }

    #[test]
    fn handle_vs_email_boundary() {
        let o = extract("@alice met alice@example.com and @bob");
        let handles: Vec<_> = o
            .entities
            .iter()
            .filter(|e| e.kind == EntityKind::Handle)
            .map(|e| e.text.as_str())
            .collect();
        let emails: Vec<_> = o
            .entities
            .iter()
            .filter(|e| e.kind == EntityKind::Email)
            .map(|e| e.text.as_str())
            .collect();
        assert_eq!(handles, vec!["alice", "bob"]);
        assert_eq!(emails, vec!["alice@example.com"]);
    }

    #[test]
    fn discord_style_handle() {
        let o = extract("ping alice#1234");
        let h: Vec<_> = o
            .entities
            .iter()
            .filter(|e| e.kind == EntityKind::Handle)
            .collect();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].text, "alice#1234");
    }

    #[test]
    fn hashtag_emits_topic() {
        let o = extract("tracking #launch-q2 updates");
        assert_eq!(
            o.entities
                .iter()
                .filter(|e| e.kind == EntityKind::Hashtag)
                .count(),
            1
        );
        assert_eq!(o.topics.len(), 1);
        assert_eq!(o.topics[0].label, "launch-q2");
    }

    #[test]
    fn hashtag_requires_leading_letter() {
        let o = extract("#123 no, #x1 yes");
        let tags: Vec<_> = o
            .entities
            .iter()
            .filter(|e| e.kind == EntityKind::Hashtag)
            .collect();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].text, "x1");
    }

    #[test]
    fn utf8_span_is_char_not_byte() {
        let o = extract("中 a@b.com");
        let email = o
            .entities
            .iter()
            .find(|e| e.kind == EntityKind::Email)
            .unwrap();
        assert_eq!(email.span_start, 2);
    }

    #[test]
    fn all_mechanical_kinds_in_one_pass() {
        let o = extract("email a@b.com, url https://x.com, @alice, #topic1");
        let k = kinds(&o);
        assert!(k.contains(&EntityKind::Email));
        assert!(k.contains(&EntityKind::Url));
        assert!(k.contains(&EntityKind::Handle));
        assert!(k.contains(&EntityKind::Hashtag));
    }

    #[test]
    fn scores_always_one() {
        let o = extract("a@b.com #x @y https://q.com");
        for e in &o.entities {
            assert!((e.score - 1.0).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn empty_input_no_matches() {
        let o = extract("plain prose with no identifiers");
        assert!(o.entities.is_empty());
    }
}
