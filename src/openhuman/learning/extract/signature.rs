//! Email signature parser — Phase 2 producer for Identity candidates.
//!
//! Subscribes to [`DomainEvent::DocumentCanonicalized`] events whose
//! `source_kind == "email"` and parses the trailing signature region of the
//! email body for identity facets (name, role, timezone, employer, location).
//!
//! ## Design notes
//!
//! The parser is intentionally conservative: false positives would pollute the
//! identity class with noisy data and erode user trust. Each detection rule
//! requires a concrete structural signal (capitalised name pattern, role keyword
//! anchor, timezone abbreviation, etc.) rather than scoring free text.
//!
//! ## Registration
//!
//! Call [`register_email_signature_subscriber`] once at startup (alongside the
//! `TracingSubscriber` and other domain subscribers). The returned
//! `SubscriptionHandle` must be kept alive for the subscriber to remain active.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;

use crate::core::event_bus::{subscribe_global, DomainEvent, EventHandler, SubscriptionHandle};
use crate::openhuman::learning::candidate::{
    self, CueFamily, EvidenceRef, FacetClass, LearningCandidate,
};

// ── Constants ────────────────────────────────────────────────────────────────

/// Number of non-empty lines to scan from the bottom of the body.
const SIG_WINDOW_LINES: usize = 8;

/// Initial confidence values per detection kind.
const CONF_NAME: f64 = 0.85;
const CONF_ROLE: f64 = 0.80;
const CONF_TIMEZONE: f64 = 0.90;
const CONF_EMPLOYER: f64 = 0.70;
const CONF_LOCATION: f64 = 0.60;

// ── Public parse function ────────────────────────────────────────────────────

/// Parse the trailing signature region of `email_body` for identity facets.
///
/// Returns a (possibly empty) list of [`LearningCandidate`]s, one per
/// detected signal. Candidates with `initial_confidence` < 0.6 are
/// dropped before returning.
///
/// # Arguments
///
/// * `email_body` — the canonical markdown body of the email message
/// * `source_id` — the ingest source id (e.g. `"gmail:abc"`)
/// * `message_id` — provider message identifier for provenance
pub fn parse_signature(
    email_body: &str,
    source_id: &str,
    message_id: &str,
) -> Vec<LearningCandidate> {
    let now = now_secs();

    // Take the last SIG_WINDOW_LINES non-empty lines in original order.
    let all_non_empty: Vec<&str> = email_body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    let start = all_non_empty.len().saturating_sub(SIG_WINDOW_LINES);
    let sig_lines = &all_non_empty[start..];

    if sig_lines.is_empty() {
        return Vec::new();
    }

    let evidence = || EvidenceRef::EmailMessage {
        source_id: source_id.to_string(),
        message_id: message_id.to_string(),
    };

    let mut candidates: Vec<LearningCandidate> = Vec::new();

    // Track whether we found any strong signature signal so we can gate
    // low-confidence detections (location).
    let mut strong_signal_found = false;

    // --- Name detection ---
    let mut name_line_idx: Option<usize> = None;
    for (idx, line) in sig_lines.iter().enumerate() {
        let t = line.trim();
        if is_likely_name(t) {
            name_line_idx = Some(idx);
            candidates.push(LearningCandidate {
                class: FacetClass::Identity,
                key: "name".to_string(),
                value: t.to_string(),
                cue_family: CueFamily::Structural,
                evidence: evidence(),
                initial_confidence: CONF_NAME,
                observed_at: now,
            });
            strong_signal_found = true;
            tracing::debug!(
                "[learning::extract::signature] name detected: {:?} source_id={}",
                t,
                source_id
            );
            break;
        }
    }

    // --- Role detection ---
    let mut role_line_idx: Option<usize> = None;
    for (idx, line) in sig_lines.iter().enumerate() {
        let t = line.trim();
        if let Some(role) = extract_role(t) {
            role_line_idx = Some(idx);
            candidates.push(LearningCandidate {
                class: FacetClass::Identity,
                key: "role".to_string(),
                value: role.to_string(),
                cue_family: CueFamily::Structural,
                evidence: evidence(),
                initial_confidence: CONF_ROLE,
                observed_at: now,
            });
            strong_signal_found = true;
            tracing::debug!(
                "[learning::extract::signature] role detected: {:?} source_id={}",
                role,
                source_id
            );
            break;
        }
    }

    // --- Timezone detection ---
    for line in sig_lines {
        let t = line.trim();
        if let Some(tz) = extract_timezone(t) {
            candidates.push(LearningCandidate {
                class: FacetClass::Identity,
                key: "timezone".to_string(),
                value: tz.to_string(),
                cue_family: CueFamily::Structural,
                evidence: evidence(),
                initial_confidence: CONF_TIMEZONE,
                observed_at: now,
            });
            strong_signal_found = true;
            tracing::debug!(
                "[learning::extract::signature] timezone detected: {:?} source_id={}",
                tz,
                source_id
            );
            break;
        }
    }

    // --- Employer detection ---
    // Strategy 1: line immediately following the role line.
    if let Some(role_idx) = role_line_idx {
        let employer_idx = role_idx + 1;
        if employer_idx < sig_lines.len() {
            let t = sig_lines[employer_idx].trim();
            if is_plausible_employer(t) {
                candidates.push(LearningCandidate {
                    class: FacetClass::Identity,
                    key: "employer".to_string(),
                    value: clean_employer(t),
                    cue_family: CueFamily::Structural,
                    evidence: evidence(),
                    initial_confidence: CONF_EMPLOYER,
                    observed_at: now,
                });
                strong_signal_found = true;
                tracing::debug!(
                    "[learning::extract::signature] employer (post-role) detected: {:?} source_id={}",
                    t,
                    source_id
                );
            }
        }
    }
    // Strategy 2: "@ Company" or "Company, Inc" pattern on any sig line.
    if !candidates.iter().any(|c| c.key == "employer") {
        for line in sig_lines {
            let t = line.trim();
            if let Some(emp) = extract_employer_pattern(t) {
                candidates.push(LearningCandidate {
                    class: FacetClass::Identity,
                    key: "employer".to_string(),
                    value: emp,
                    cue_family: CueFamily::Structural,
                    evidence: evidence(),
                    initial_confidence: CONF_EMPLOYER,
                    observed_at: now,
                });
                strong_signal_found = true;
                tracing::debug!(
                    "[learning::extract::signature] employer (pattern) detected source_id={}",
                    source_id
                );
                break;
            }
        }
    }

    // --- Location detection (low-confidence, only when strong signals present) ---
    if strong_signal_found {
        // Look at the line right after the name line.
        if let Some(name_idx) = name_line_idx {
            let loc_idx = name_idx + 1;
            if loc_idx < sig_lines.len() {
                let t = sig_lines[loc_idx].trim();
                // Skip if already matched as role or employer.
                let already_used = role_line_idx == Some(loc_idx);
                if !already_used {
                    if let Some(loc) = extract_location(t) {
                        candidates.push(LearningCandidate {
                            class: FacetClass::Identity,
                            key: "location".to_string(),
                            value: loc,
                            cue_family: CueFamily::Structural,
                            evidence: evidence(),
                            initial_confidence: CONF_LOCATION,
                            observed_at: now,
                        });
                        tracing::debug!(
                            "[learning::extract::signature] location detected source_id={}",
                            source_id
                        );
                    }
                }
            }
        }
    }

    // Drop low-confidence noise. All CONF_* constants are currently ≥ 0.6,
    // so this is mostly a guard against future regressions and codifies the
    // contract advertised in the doc comment.
    let kept = candidates.len();
    candidates.retain(|c| c.initial_confidence >= 0.6);
    let dropped = kept - candidates.len();

    tracing::debug!(
        "[learning::extract::signature] parse_signature source_id={} candidates={} dropped={}",
        source_id,
        candidates.len(),
        dropped,
    );

    candidates
}

// ── Detection helpers ─────────────────────────────────────────────────────────

/// Returns `true` if `s` looks like a person's name: 1–4 capitalised words,
/// no `@`, no URL fragments, mostly alphabetic, no trailing punctuation like commas.
fn is_likely_name(s: &str) -> bool {
    if s.contains('@') || s.contains("://") || s.contains("www.") {
        return false;
    }
    // Lines ending with punctuation like "Thanks," or "Best regards," are not names.
    if s.ends_with(',') || s.ends_with(':') || s.ends_with(';') {
        return false;
    }
    // Lines that are clearly greetings or sign-offs.
    let lower = s.to_ascii_lowercase();
    const EXCLUSIONS: &[&str] = &[
        "thanks",
        "regards",
        "best",
        "cheers",
        "sincerely",
        "cordially",
        "hi",
        "hello",
        "dear",
        "hey",
    ];
    if EXCLUSIONS.iter().any(|e| lower == *e) {
        return false;
    }

    let words: Vec<&str> = s.split_whitespace().collect();
    if words.is_empty() || words.len() > 4 {
        return false;
    }
    // Every word must start with an uppercase letter and be mostly alpha.
    for w in &words {
        let first = w.chars().next().unwrap_or(' ');
        if !first.is_uppercase() {
            return false;
        }
        // Strip trailing punctuation for the ratio check.
        let clean: String = w.chars().filter(|c| c.is_alphabetic()).collect();
        if clean.is_empty() {
            return false;
        }
        let alpha_ratio = clean.len() as f64 / w.len() as f64;
        if alpha_ratio < 0.7 {
            return false;
        }
    }
    // Must contain at least two words or exactly one that's clearly a name
    // (more than 3 chars). Single-word uppercase abbreviations like "PST"
    // would be false positives without this guard.
    words.len() >= 2 || (words.len() == 1 && words[0].len() > 3)
}

/// Extracts a role string from `s` if it contains a known role keyword.
/// Returns the trimmed line (or the keyword match) as the role value.
fn extract_role(s: &str) -> Option<&str> {
    const ROLE_KEYWORDS: &[&str] = &[
        "engineer",
        "developer",
        "designer",
        "manager",
        "director",
        "founder",
        "cto",
        "ceo",
        "coo",
        "cpo",
        "cfo",
        "product manager",
        "consultant",
        "lead",
        "head of",
        "vp",
        "vice president",
        "principal",
        "staff ",
        "senior ",
        "architect",
        "researcher",
        "analyst",
        "recruiter",
        "sales",
        "marketing",
        "intern",
    ];
    let lower = s.to_ascii_lowercase();
    for kw in ROLE_KEYWORDS {
        if lower.contains(kw) {
            return Some(s);
        }
    }
    None
}

/// Extracts a timezone abbreviation from `s`.
fn extract_timezone(s: &str) -> Option<&str> {
    // Scan for known timezone patterns. We return the matched slice.
    const STATIC_TZ: &[&str] = &[
        "PST", "PDT", "EST", "EDT", "MST", "MDT", "CST", "CDT", "UTC", "GMT",
    ];
    for tz in STATIC_TZ {
        if let Some(pos) = s.find(tz) {
            // Confirm the match is a whole word (preceded/followed by non-alpha).
            let before_ok = pos == 0 || !s[..pos].ends_with(|c: char| c.is_alphabetic());
            let after = &s[pos + tz.len()..];
            let after_ok = after.is_empty()
                || after.starts_with(|c: char| !c.is_alphabetic())
                || after.starts_with(['+', '-']);
            if before_ok && after_ok {
                // Grab "UTC+5:30" or "GMT-7" style suffix.
                if tz.starts_with("UTC") || tz.starts_with("GMT") {
                    let end = s[pos + tz.len()..]
                        .find(|c: char| !c.is_ascii_digit() && c != '+' && c != '-' && c != ':')
                        .map(|off| pos + tz.len() + off)
                        .unwrap_or(s.len());
                    return Some(&s[pos..end]);
                }
                return Some(&s[pos..pos + tz.len()]);
            }
        }
    }
    None
}

/// Returns `true` if `s` is plausible as a company / employer name line
/// (not an email, not a URL, contains at least one capital letter, not pure punctuation).
fn is_plausible_employer(s: &str) -> bool {
    if s.is_empty() || s.len() > 80 {
        return false;
    }
    if s.contains('@') || s.contains("://") || s.contains("www.") {
        return false;
    }
    // Must have at least one uppercase letter.
    if !s.chars().any(|c| c.is_uppercase()) {
        return false;
    }
    let alpha_count = s.chars().filter(|c| c.is_alphabetic()).count();
    alpha_count >= 2
}

/// Cleans a raw employer line: trims, strips trailing punctuation.
fn clean_employer(s: &str) -> String {
    s.trim()
        .trim_end_matches([',', '.', ';'])
        .trim()
        .to_string()
}

/// Tries to extract an employer from patterns: `"@ Company"` or `"Company, Inc"`.
fn extract_employer_pattern(s: &str) -> Option<String> {
    let t = s.trim();
    // "@ Company Name"
    if let Some(stripped) = t.strip_prefix('@') {
        let name = stripped.trim();
        if !name.is_empty() && !name.contains('@') {
            return Some(name.to_string());
        }
    }
    // "Company, Inc" / "Company LLC" / "Company Ltd"
    let lower = t.to_ascii_lowercase();
    let corp_suffixes = [
        ", inc", " inc.", " llc", " ltd", " limited", " corp", " co.",
    ];
    for suffix in corp_suffixes {
        if (lower.ends_with(suffix) || lower.contains(&format!("{suffix} ")))
            && is_plausible_employer(t)
        {
            return Some(clean_employer(t));
        }
    }
    None
}

/// Attempts to extract a city/state location from a line.
/// Very conservative: requires a comma between city and state/country-like fragment.
fn extract_location(s: &str) -> Option<String> {
    let t = s.trim();
    if t.contains('@') || t.contains("://") {
        return None;
    }
    // Simple heuristic: "City, ST" or "City, Country"
    if let Some(comma_pos) = t.find(',') {
        let city = t[..comma_pos].trim();
        let region = t[comma_pos + 1..].trim();
        // City: 2-30 chars, no digits, starts with uppercase.
        let city_ok = city.len() >= 2
            && city.len() <= 30
            && city.chars().next().is_some_and(|c| c.is_uppercase())
            && !city.chars().any(|c| c.is_ascii_digit());
        // Region: 2-20 chars.
        let region_ok = region.len() >= 2 && region.len() <= 20;
        if city_ok && region_ok {
            return Some(t.to_string());
        }
    }
    None
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// ── Subscriber ───────────────────────────────────────────────────────────────

/// Event subscriber that reacts to `DocumentCanonicalized` events for email
/// sources and routes detected identity candidates into the global buffer.
pub struct EmailSignatureSubscriber;

#[async_trait]
impl EventHandler for EmailSignatureSubscriber {
    fn name(&self) -> &str {
        "learning::extract::signature"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["memory"])
    }

    async fn handle(&self, event: &DomainEvent) {
        if let DomainEvent::DocumentCanonicalized {
            source_id,
            source_kind,
            body_preview,
            chunk_ids,
            ..
        } = event
        {
            if source_kind != "email" {
                return;
            }
            let body = match body_preview {
                Some(b) => b,
                None => {
                    tracing::debug!(
                        "[learning::extract::signature] no body_preview on DocumentCanonicalized \
                         source_id={} — skipping signature parse",
                        source_id
                    );
                    return;
                }
            };

            // Use the first chunk_id as the message_id if no dedicated field is
            // available. The EmailMessage evidence variant carries both the
            // source_id and message_id for provenance.
            let message_id = chunk_ids
                .first()
                .cloned()
                .unwrap_or_else(|| source_id.clone());

            tracing::debug!(
                "[learning::extract::signature] parsing email signature source_id={} body_len={}",
                source_id,
                body.len()
            );

            let candidates = parse_signature(body, source_id, &message_id);
            let count = candidates.len();
            let buf = candidate::global();
            for c in candidates {
                buf.push(c);
            }
            tracing::debug!(
                "[learning::extract::signature] pushed {} identity candidate(s) for source_id={}",
                count,
                source_id
            );
        }
    }
}

/// Register the email signature subscriber on the global event bus.
///
/// Must be called at startup after [`crate::core::event_bus::init_global`].
/// The returned handle keeps the subscription alive — store it in a long-lived
/// container (e.g. alongside other `SubscriptionHandle`s in startup).
pub fn register_email_signature_subscriber() -> Option<SubscriptionHandle> {
    subscribe_global(Arc::new(EmailSignatureSubscriber))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod signature_tests {
    use super::*;
    use crate::openhuman::learning::candidate::{CueFamily, EvidenceRef, FacetClass};

    fn extract(body: &str) -> Vec<LearningCandidate> {
        parse_signature(body, "gmail:test", "<msg-1@test.com>")
    }

    fn find<'a>(cs: &'a [LearningCandidate], key: &str) -> Option<&'a LearningCandidate> {
        cs.iter().find(|c| c.key == key)
    }

    #[test]
    fn parse_signature_extracts_name_role_timezone_employer() {
        let body = "Hi, great to hear from you!\n\n\
                    Please find the docs attached.\n\n\
                    Thanks,\n\
                    Alice Johnson\n\
                    Senior Software Engineer\n\
                    Acme Corp\n\
                    San Francisco, CA\n\
                    PST";
        let candidates = extract(body);

        let name = find(&candidates, "name").expect("name candidate");
        assert_eq!(name.value, "Alice Johnson");
        assert!((name.initial_confidence - CONF_NAME).abs() < 0.01);

        let role = find(&candidates, "role").expect("role candidate");
        assert!(role.value.contains("Engineer") || role.value.contains("engineer"));

        let tz = find(&candidates, "timezone").expect("timezone candidate");
        assert_eq!(tz.value, "PST");
        assert!((tz.initial_confidence - CONF_TIMEZONE).abs() < 0.01);

        let emp = find(&candidates, "employer").expect("employer candidate");
        assert!(emp.value.contains("Acme"));
    }

    #[test]
    fn parse_signature_handles_no_signature() {
        let body = "just some content here nothing looks like a sig";
        let cs = extract(body);
        // No strong signals → no candidates emitted. The < 0.6 filter at the
        // tail of parse_signature drops anything below confidence threshold
        // (including the gated lone-location case), so the result must be empty.
        assert!(
            cs.is_empty(),
            "expected zero candidates from non-signature body, got {cs:?}"
        );
    }

    #[test]
    fn parse_signature_ignores_quoted_replies() {
        // Even when the body has quoted sections, only the window of non-empty lines
        // at the very end is scanned.
        let body = "> On Monday, Alice wrote:\n\
                    > Great to meet you!\n\
                    >\n\
                    Sure, let's connect.\n\n\
                    Bob Smith\n\
                    Product Manager\n\
                    UTC+1";
        let cs = extract(body);
        let name = find(&cs, "name").expect("name candidate");
        assert_eq!(name.value, "Bob Smith");
        let tz = find(&cs, "timezone").expect("timezone");
        assert!(tz.value.starts_with("UTC"));
    }

    #[test]
    fn parse_signature_low_confidence_for_lone_location() {
        // Location alone (no other strong signals) should NOT produce a candidate.
        let body = "The meeting is on Thursday.\nSan Francisco, CA";
        let cs = extract(body);
        assert!(
            find(&cs, "location").is_none(),
            "location candidate must not be emitted without other strong signals"
        );
    }

    #[test]
    fn parse_signature_emits_evidence_email_message_variant() {
        let body = "Alice Smith\nCTO\nStartup Inc\nPST";
        let cs = extract(body);
        for c in &cs {
            assert!(
                matches!(
                    &c.evidence,
                    EvidenceRef::EmailMessage { source_id, message_id }
                    if source_id == "gmail:test" && message_id == "<msg-1@test.com>"
                ),
                "expected EmailMessage evidence, got {:?}",
                c.evidence
            );
            assert_eq!(c.cue_family, CueFamily::Structural);
            assert_eq!(c.class, FacetClass::Identity);
        }
    }
}
