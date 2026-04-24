//! SKILL.md body injection into the agent inference loop.
//!
//! This module wires the installed `SKILL.md` catalog into each user
//! turn so the LLM can see a matched skill's instruction body in
//! context. The plain-text catalog section that the prompt builder
//! already renders (`## Available Skills` — name + description only)
//! tells the model **what** skills exist; this injection step gives it
//! the actual instruction bodies for the specific skill(s) relevant to
//! the current message.
//!
//! ## Matching heuristic (v1)
//!
//! For each skill we emit a `matched` decision:
//!
//! 1. **Explicit `@<skill-name>` mention** in the user message — always
//!    force-injects. Takes precedence over everything else. Names are
//!    matched case-insensitively; `@foo bar` matches skill name
//!    `foo-bar` after normalising `-`/`_`/whitespace → `_`.
//! 2. Otherwise, when the skill does **not** declare
//!    `user-invocable: false` (default = invocable = true):
//!    - `matched = true` when the skill's `description` appears as a
//!      case-insensitive substring of the user message, OR any of its
//!      `tags` appears as a whole-word case-insensitive substring, OR
//!      the skill's `name` appears as a whole-word match.
//! 3. Skills with `user-invocable: false` **only** ever inject on an
//!    explicit `@` mention — the auto-match path is disabled for them.
//!
//! The heuristic is intentionally narrow: exact + case-insensitive
//! substring is cheap, predictable for reviewers, and keeps false
//! positives bounded by the 8 KiB total injected-byte cap enforced
//! downstream in [`render_injection`]. More sophisticated ranking
//! (embeddings, LLM-rerank) can replace this later without touching
//! the calling site in `Agent::turn`.
//!
//! ## Ordering
//!
//! Matched skills are returned in this stable order:
//!
//! 1. Explicit `@` mentions in the order they appear in the message.
//! 2. Auto-matched skills by description length (longer first), then
//!    by skill name alphabetically as a deterministic tiebreaker.
//!
//! ## Size cap
//!
//! Total injected payload (sum of all `[SKILL:<name>] … [/SKILL]`
//! blocks) is capped at [`DEFAULT_MAX_INJECTION_BYTES`] = 8 KiB. When
//! a single body would push the total over the cap, it is truncated
//! and a `[SKILL:<name>:truncated]` marker replaces the closer so the
//! LLM knows the content was cut short. Any subsequent matched skills
//! that would exceed the cap are skipped with `SkipReason::BudgetExhausted`
//! and logged.
//!
//! ## Logging
//!
//! Every candidate emits a grep-friendly `[skills:inject]` log line
//! with `matched=<bool>`, reason, and injected bytes (see
//! [`render_injection`]). A summary line lives in the caller
//! (`Agent::turn`).

use super::Skill;
use std::collections::HashSet;

/// Upper bound on total bytes injected per turn. Matches the umbrella
/// issue #781 acceptance criterion ("≤ 8 KiB").
pub const DEFAULT_MAX_INJECTION_BYTES: usize = 8 * 1024;

/// Why a candidate skill was skipped. Kept on the match record for
/// both logging and unit-test assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// `user-invocable: false` skill without an explicit `@` mention.
    NotUserInvocable,
    /// No match in description / tags / name, and no `@` mention.
    NoMatch,
    /// Skill body could not be read from disk (legacy manifest or I/O
    /// failure).
    BodyUnavailable,
    /// Skill body would push the running total past the size cap.
    BudgetExhausted,
}

/// How a matched skill was selected. Preserved on `SkillMatch` so the
/// logger can explain *why* each injection happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchReason {
    /// Selected via an explicit `@<skill-name>` mention.
    AtMention,
    /// Description substring matched the user message.
    DescriptionSubstring,
    /// A tag matched as a whole-word substring.
    TagMatch,
    /// The skill name itself appeared in the message.
    NameMatch,
}

impl MatchReason {
    fn as_str(self) -> &'static str {
        match self {
            MatchReason::AtMention => "at_mention",
            MatchReason::DescriptionSubstring => "description_substring",
            MatchReason::TagMatch => "tag_match",
            MatchReason::NameMatch => "name_match",
        }
    }
}

/// A skill that passed the matcher. The caller resolves its body at
/// render time.
#[derive(Debug, Clone)]
pub struct SkillMatch<'a> {
    pub skill: &'a Skill,
    pub reason: MatchReason,
    /// Position in the user message for `@`-mention matches. Used to
    /// preserve message order. Auto-matches get `usize::MAX` so they
    /// sort after explicit mentions.
    pub mention_index: usize,
}

/// Per-skill decision returned to the caller for logging. Covers both
/// matched and skipped candidates so there is a single source of truth
/// for what happened this turn.
#[derive(Debug, Clone)]
pub struct SkillDecision {
    pub name: String,
    pub matched: bool,
    pub reason: String,
    pub injected_bytes: usize,
    pub truncated: bool,
}

/// Result of [`render_injection`] — the rendered block plus machine-
/// readable stats for logging.
#[derive(Debug, Clone, Default)]
pub struct Injection {
    /// Concatenated `[SKILL:<name>] … [/SKILL]` blocks. Empty when
    /// nothing matched (or every match was skipped).
    pub rendered: String,
    /// Total bytes in `rendered`.
    pub injected_bytes: usize,
    /// Whether at least one body was truncated to fit the cap.
    pub truncated: bool,
    /// Per-candidate decisions (both matched and skipped) for logging.
    pub decisions: Vec<SkillDecision>,
}

/// Read the `user-invocable` flag from a skill's frontmatter. Defaults
/// to `true` (opt-out) when absent or unparseable. Accepts both the
/// spec-compliant `metadata.user-invocable` location and the deprecated
/// top-level `user-invocable` key (emitted with a migration warning by
/// the catalog loader).
pub fn is_user_invocable(skill: &Skill) -> bool {
    let lookup_bool = |key: &str| -> Option<bool> {
        if let Some(v) = skill.frontmatter.metadata.get(key) {
            if let Some(b) = v.as_bool() {
                return Some(b);
            }
        }
        if let Some(v) = skill.frontmatter.extra.get(key) {
            if let Some(b) = v.as_bool() {
                return Some(b);
            }
        }
        None
    };
    lookup_bool("user-invocable")
        .or_else(|| lookup_bool("user_invocable"))
        .unwrap_or(true)
}

/// Normalise a skill name for case-insensitive `@` matching:
/// lowercase, collapse `-`/`_` runs to single `-`.
fn normalise(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_sep = false;
    for ch in name.chars().flat_map(|c| c.to_lowercase()) {
        if ch == '-' || ch == '_' {
            if !prev_sep && !out.is_empty() {
                out.push('-');
            }
            prev_sep = true;
        } else {
            out.push(ch);
            prev_sep = false;
        }
    }
    // Trim trailing separator if any.
    if out.ends_with('-') {
        out.pop();
    }
    out
}

/// Scan the user message for `@<skill-name>` patterns. Returns the
/// normalised skill name plus the byte index at which the `@` appears
/// (used later to preserve the original message order across mentions).
///
/// A token qualifies as an `@` mention when:
/// - it starts with `@` (not preceded by an alphanumeric character so
///   email addresses don't accidentally trigger)
/// - and the following run of `[A-Za-z0-9_-]+` is non-empty
pub fn extract_mentions(user_message: &str) -> Vec<(String, usize)> {
    let bytes = user_message.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            let preceded_by_alnum = i > 0
                && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'.')
                && !bytes[i - 1].is_ascii_whitespace();
            if preceded_by_alnum {
                i += 1;
                continue;
            }
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() {
                let c = bytes[end];
                if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' {
                    end += 1;
                } else {
                    break;
                }
            }
            if end > start {
                let name = &user_message[start..end];
                out.push((normalise(name), i));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn contains_whole_word(haystack_lower: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return false;
    }
    // Whole-word = surrounding chars are NOT alphanumeric/_-. Simple
    // loop over match positions rather than pulling in a regex crate.
    let hay = haystack_lower.as_bytes();
    let ndl = needle_lower.as_bytes();
    if ndl.len() > hay.len() {
        return false;
    }
    // `@` counts as a word character so a name/tag that happens to sit
    // inside an email or `@mention` (`foo@alice.example.com`, `@gmail`)
    // does not slip through the whole-word gate. Explicit mentions are
    // handled separately by [`extract_mentions`].
    let is_word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b'@';
    let mut i = 0;
    while i + ndl.len() <= hay.len() {
        if &hay[i..i + ndl.len()] == ndl {
            let left_ok = i == 0 || !is_word(hay[i - 1]);
            let right_ok = i + ndl.len() == hay.len() || !is_word(hay[i + ndl.len()]);
            if left_ok && right_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Match installed skills against a user message per the heuristic
/// documented at the top of this module.
pub fn match_skills<'a>(skills: &'a [Skill], user_message: &str) -> Vec<SkillMatch<'a>> {
    let mentions = extract_mentions(user_message);
    let mention_set: HashSet<String> = mentions.iter().map(|(n, _)| n.clone()).collect();
    let mention_index = |skill_norm: &str| -> Option<usize> {
        mentions
            .iter()
            .find(|(n, _)| n == skill_norm)
            .map(|(_, idx)| *idx)
    };

    let lower_msg = user_message.to_lowercase();

    let mut matches: Vec<SkillMatch<'a>> = Vec::new();
    for skill in skills {
        let normalised_name = normalise(&skill.name);
        let user_invocable = is_user_invocable(skill);

        // 1. `@` mention always wins.
        if mention_set.contains(&normalised_name) {
            let idx = mention_index(&normalised_name).unwrap_or(usize::MAX);
            matches.push(SkillMatch {
                skill,
                reason: MatchReason::AtMention,
                mention_index: idx,
            });
            continue;
        }

        // 2. Auto-match only when skill allows user invocation.
        if !user_invocable {
            continue;
        }

        let desc_lower = skill.description.to_lowercase();
        if !desc_lower.is_empty() && lower_msg.contains(&desc_lower) {
            matches.push(SkillMatch {
                skill,
                reason: MatchReason::DescriptionSubstring,
                mention_index: usize::MAX,
            });
            continue;
        }

        let mut tag_hit = false;
        for tag in &skill.tags {
            let tag_lower = tag.to_lowercase();
            if contains_whole_word(&lower_msg, &tag_lower) {
                tag_hit = true;
                break;
            }
        }
        if tag_hit {
            matches.push(SkillMatch {
                skill,
                reason: MatchReason::TagMatch,
                mention_index: usize::MAX,
            });
            continue;
        }

        // Name-as-whole-word fallback (e.g. user says "run the
        // pdf-cruncher skill"). Skipped when the name is a very short
        // token that would over-match (<= 2 chars).
        let name_lower = skill.name.to_lowercase();
        if name_lower.chars().count() > 2 && contains_whole_word(&lower_msg, &name_lower) {
            matches.push(SkillMatch {
                skill,
                reason: MatchReason::NameMatch,
                mention_index: usize::MAX,
            });
        }
    }

    // Stable ordering: `@` mentions by message index first; auto-matches
    // by description length descending, tie-breaking on skill name.
    matches.sort_by(|a, b| match (a.reason, b.reason) {
        (MatchReason::AtMention, MatchReason::AtMention) => a.mention_index.cmp(&b.mention_index),
        (MatchReason::AtMention, _) => std::cmp::Ordering::Less,
        (_, MatchReason::AtMention) => std::cmp::Ordering::Greater,
        _ => {
            let len_cmp = b.skill.description.len().cmp(&a.skill.description.len());
            if len_cmp != std::cmp::Ordering::Equal {
                len_cmp
            } else {
                a.skill.name.cmp(&b.skill.name)
            }
        }
    });

    matches
}

/// Build the injection block. Resolves each match's body via
/// `body_resolver` so callers can swap in a fake reader for tests.
///
/// `max_bytes` caps the total rendered size. When a body would exceed
/// the remaining budget it is truncated on a UTF-8 boundary and
/// emitted with a `[SKILL:<name>:truncated]` close marker.
pub fn render_injection<'a, F>(
    matches: &[SkillMatch<'a>],
    max_bytes: usize,
    mut body_resolver: F,
) -> Injection
where
    F: FnMut(&Skill) -> Option<String>,
{
    const SKILL_OPEN_FMT: &str = "[SKILL:{}]\n";
    const SKILL_CLOSE_FMT: &str = "\n[/SKILL]\n";
    const SKILL_CLOSE_TRUNC_FMT: &str = "\n[/SKILL:truncated]\n";

    let mut rendered = String::new();
    let mut decisions: Vec<SkillDecision> = Vec::new();
    let mut truncated_any = false;

    for m in matches {
        let name = &m.skill.name;
        let body = match body_resolver(m.skill) {
            Some(b) => b,
            None => {
                log::warn!(
                    "[skills:inject] matched={} reason={} name={} skipped=body_unavailable",
                    false,
                    "body_unavailable",
                    name
                );
                decisions.push(SkillDecision {
                    name: name.clone(),
                    matched: false,
                    reason: format!("skipped:{:?}", SkipReason::BodyUnavailable),
                    injected_bytes: 0,
                    truncated: false,
                });
                continue;
            }
        };

        let header = SKILL_OPEN_FMT.replacen("{}", name, 1);
        let footer_full = SKILL_CLOSE_FMT.to_string();
        let footer_trunc = SKILL_CLOSE_TRUNC_FMT.to_string();

        let remaining = max_bytes.saturating_sub(rendered.len());
        let header_len = header.len();
        let footer_full_len = footer_full.len();
        let footer_trunc_len = footer_trunc.len();

        // Minimum we need to emit anything meaningful: header + at
        // least 1 byte of body + truncation footer.
        let min_truncated = header_len + footer_trunc_len + 1;
        if remaining < min_truncated {
            log::info!(
                "[skills:inject] matched={} reason={} name={} skipped=budget_exhausted remaining_bytes={}",
                false,
                "budget_exhausted",
                name,
                remaining
            );
            decisions.push(SkillDecision {
                name: name.clone(),
                matched: false,
                reason: format!("skipped:{:?}", SkipReason::BudgetExhausted),
                injected_bytes: 0,
                truncated: false,
            });
            continue;
        }

        // Can we fit the whole body + full footer?
        let full_len = header_len + body.len() + footer_full_len;
        if full_len <= remaining {
            rendered.push_str(&header);
            rendered.push_str(&body);
            rendered.push_str(&footer_full);
            let injected = header_len + body.len() + footer_full_len;
            log::debug!(
                "[skills:inject] matched={} reason={} name={} injected_bytes={} truncated={}",
                true,
                m.reason.as_str(),
                name,
                injected,
                false
            );
            decisions.push(SkillDecision {
                name: name.clone(),
                matched: true,
                reason: m.reason.as_str().to_string(),
                injected_bytes: injected,
                truncated: false,
            });
            continue;
        }

        // Truncate: how many body bytes can we fit with the truncated
        // footer?
        let max_body = remaining.saturating_sub(header_len + footer_trunc_len);
        // Round down to a char boundary.
        let mut cut = max_body.min(body.len());
        while cut > 0 && !body.is_char_boundary(cut) {
            cut -= 1;
        }
        let truncated_body = &body[..cut];

        rendered.push_str(&header);
        rendered.push_str(truncated_body);
        rendered.push_str(&footer_trunc);
        truncated_any = true;
        let injected = header_len + truncated_body.len() + footer_trunc_len;
        log::warn!(
            "[skills:inject] matched={} reason={} name={} injected_bytes={} truncated={} body_bytes_total={} body_bytes_kept={}",
            true,
            m.reason.as_str(),
            name,
            injected,
            true,
            body.len(),
            truncated_body.len()
        );
        decisions.push(SkillDecision {
            name: name.clone(),
            matched: true,
            reason: m.reason.as_str().to_string(),
            injected_bytes: injected,
            truncated: true,
        });
    }

    let injected_bytes = rendered.len();
    Injection {
        rendered,
        injected_bytes,
        truncated: truncated_any,
        decisions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::skills::{Skill, SkillFrontmatter};
    use std::collections::HashMap;

    fn skill(name: &str, description: &str) -> Skill {
        Skill {
            name: name.to_string(),
            dir_name: name.to_string(),
            description: description.to_string(),
            version: "0.1.0".into(),
            author: None,
            tags: Vec::new(),
            tools: Vec::new(),
            prompts: Vec::new(),
            location: None,
            frontmatter: SkillFrontmatter::default(),
            resources: Vec::new(),
            scope: Default::default(),
            legacy: false,
            warnings: Vec::new(),
        }
    }

    fn skill_with_tags(name: &str, description: &str, tags: &[&str]) -> Skill {
        let mut s = skill(name, description);
        s.tags = tags.iter().map(|t| t.to_string()).collect();
        s
    }

    fn skill_with_flag(name: &str, description: &str, flag_key: &str, flag: bool) -> Skill {
        let mut s = skill(name, description);
        let mut map: HashMap<String, serde_yaml::Value> = HashMap::new();
        map.insert(flag_key.to_string(), serde_yaml::Value::Bool(flag));
        s.frontmatter.metadata = map;
        s
    }

    #[test]
    fn matches_skill_by_description_substring() {
        let skills = vec![skill("email", "send email via gmail")];
        let m = match_skills(&skills, "Please send email via gmail to alice.");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].reason, MatchReason::DescriptionSubstring);
    }

    #[test]
    fn matches_skill_by_tag_whole_word() {
        let skills = vec![skill_with_tags("tp", "do things", &["pdf"])];
        let m = match_skills(&skills, "Convert this pdf please.");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].reason, MatchReason::TagMatch);
    }

    #[test]
    fn tag_partial_word_does_not_match() {
        let skills = vec![skill_with_tags("sk", "x", &["crypt"])];
        let m = match_skills(&skills, "I like cryptography.");
        // `crypt` is not a standalone word in `cryptography`.
        assert!(m.is_empty(), "got: {:?}", m);
    }

    #[test]
    fn matches_skill_by_name_whole_word() {
        let skills = vec![skill("pdf-crunch", "unrelated")];
        let m = match_skills(&skills, "Run the pdf-crunch skill now");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].reason, MatchReason::NameMatch);
    }

    #[test]
    fn explicit_at_mention_force_injects() {
        let skills = vec![skill("notes", "completely unrelated description")];
        let m = match_skills(&skills, "Hey can you @notes me the summary?");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].reason, MatchReason::AtMention);
    }

    #[test]
    fn at_mention_case_insensitive_and_handles_dashes() {
        let skills = vec![skill("pdf-crunch", "foo")];
        let m = match_skills(&skills, "Use @Pdf-Crunch please");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].reason, MatchReason::AtMention);
    }

    #[test]
    fn email_address_at_does_not_trigger_mention() {
        let skills = vec![skill("alice", "nothing relevant")];
        let m = match_skills(&skills, "Send email to foo@alice.example.com please");
        // `foo@alice` should not count because `o` precedes `@`.
        assert!(m.is_empty(), "got: {:?}", m);
    }

    #[test]
    fn user_invocable_false_requires_at_mention() {
        // description contains "summarize" so it would auto-match if
        // invocable, but `user-invocable: false` blocks auto-matching.
        let skills = vec![skill_with_flag(
            "summary",
            "summarize text",
            "user-invocable",
            false,
        )];
        let m = match_skills(&skills, "Please summarize text for me.");
        assert!(m.is_empty(), "auto-match should be suppressed: {:?}", m);

        // But an explicit @ mention still force-injects.
        let m2 = match_skills(&skills, "Hey @summary for me");
        assert_eq!(m2.len(), 1);
        assert_eq!(m2[0].reason, MatchReason::AtMention);
    }

    #[test]
    fn user_invocable_deprecated_underscore_alias() {
        let skills = vec![skill_with_flag("x", "xx yy", "user_invocable", false)];
        let m = match_skills(&skills, "xx yy please");
        assert!(m.is_empty());
    }

    #[test]
    fn at_mention_overrides_non_match() {
        let skills = vec![skill("bar", "zzz unrelated")];
        let m = match_skills(&skills, "@bar do it");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].reason, MatchReason::AtMention);
    }

    #[test]
    fn longer_description_ranks_higher_on_ties() {
        let a = skill("aa", "short");
        let b = skill("bb", "this is a much longer description");
        // Both match on the word "description".
        let msg = "I want to talk about description";
        // Use tags to guarantee both match.
        let mut a = a;
        a.tags.push("description".into());
        let mut b = b;
        b.tags.push("description".into());
        let skills = [a, b];
        let m = match_skills(&skills, msg);
        assert_eq!(m.len(), 2);
        // Longer description first.
        assert_eq!(m[0].skill.name, "bb");
        assert_eq!(m[1].skill.name, "aa");
    }

    #[test]
    fn at_mentions_sort_before_auto_matches() {
        let a = skill("foo", "XXX YYY");
        let b = skill("bar", "XXX YYY");
        // `foo` auto-matches on description; `bar` is explicit via @.
        let skills = [a, b];
        let m = match_skills(&skills, "XXX YYY and @bar");
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].skill.name, "bar");
        assert_eq!(m[0].reason, MatchReason::AtMention);
    }

    #[test]
    fn render_injection_emits_full_block_when_under_budget() {
        let s = skill("hello", "say hi");
        let skills = [s];
        let matches = match_skills(&skills, "@hello please");
        let inj = render_injection(&matches, 1024, |sk| {
            assert_eq!(sk.name, "hello");
            Some("instructions body".to_string())
        });
        assert!(inj.rendered.contains("[SKILL:hello]"));
        assert!(inj.rendered.contains("instructions body"));
        assert!(inj.rendered.contains("[/SKILL]"));
        assert!(!inj.truncated);
        assert_eq!(inj.decisions.len(), 1);
        assert!(inj.decisions[0].matched);
    }

    #[test]
    fn size_cap_truncates_with_marker() {
        let s = skill("big", "huge body");
        let skills = [s];
        let matches = match_skills(&skills, "@big do it");
        // Force truncation by setting a tight cap.
        let big_body = "X".repeat(4000);
        let inj = render_injection(&matches, 200, |_| Some(big_body.clone()));
        assert!(inj.truncated, "expected truncation: {:?}", inj);
        assert!(inj.rendered.contains("[SKILL:big]"));
        assert!(inj.rendered.contains("[/SKILL:truncated]"));
        assert!(inj.injected_bytes <= 200);
        assert!(inj.decisions[0].truncated);
    }

    #[test]
    fn budget_exhausted_skips_later_candidates() {
        let a = skill("first", "x");
        let b = skill("second", "x");
        let skills = [a, b];
        let matches = match_skills(&skills, "@first @second");
        let body = "X".repeat(200);
        // Cap just big enough for one block.
        let inj = render_injection(&matches, 250, |_| Some(body.clone()));
        assert_eq!(inj.decisions.len(), 2);
        let matched_count = inj.decisions.iter().filter(|d| d.matched).count();
        assert_eq!(matched_count, 1);
        let skipped = inj.decisions.iter().find(|d| !d.matched).unwrap();
        assert!(
            skipped.reason.contains("BudgetExhausted"),
            "got: {:?}",
            skipped
        );
    }

    #[test]
    fn body_unavailable_logs_skip() {
        let s = skill("ghost", "not on disk");
        let skills = [s];
        let matches = match_skills(&skills, "@ghost");
        let inj = render_injection(&matches, 1024, |_| None);
        assert!(inj.rendered.is_empty());
        assert_eq!(inj.decisions.len(), 1);
        assert!(!inj.decisions[0].matched);
        assert!(inj.decisions[0].reason.contains("BodyUnavailable"));
    }

    #[test]
    fn legacy_skill_read_body_returns_none() {
        let mut s = skill("legacy", "d");
        s.legacy = true;
        assert!(s.read_body().is_none());
    }

    #[test]
    fn read_body_round_trip_from_tempfile() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("SKILL.md");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            "---\nname: demo\ndescription: demo skill\n---\n\nThe actual body text.\n"
        )
        .unwrap();
        drop(f);

        let mut s = skill("demo", "demo skill");
        s.location = Some(path);
        let body = s.read_body().expect("should parse body");
        assert!(body.contains("The actual body text."));
    }

    #[test]
    fn default_max_injection_bytes_matches_acceptance() {
        // The #781 acceptance criterion is a hard 8 KiB cap. Lock the
        // constant so future edits trip this test instead of silently
        // relaxing the budget.
        assert_eq!(DEFAULT_MAX_INJECTION_BYTES, 8192);
    }

    #[test]
    fn is_user_invocable_defaults_to_true() {
        let s = skill("x", "d");
        assert!(is_user_invocable(&s));
    }

    #[test]
    fn is_user_invocable_reads_extra_fallback() {
        // Deprecated top-level key lands in `extra`.
        let mut s = skill("x", "d");
        s.frontmatter
            .extra
            .insert("user-invocable".into(), serde_yaml::Value::Bool(false));
        assert!(!is_user_invocable(&s));
    }

    #[test]
    fn extract_mentions_preserves_order() {
        let m = extract_mentions("first @alpha, then @beta, then @gamma");
        let names: Vec<&str> = m.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn extract_mentions_skips_bare_at() {
        let m = extract_mentions("just an @ sign alone");
        assert!(m.is_empty(), "got: {:?}", m);
    }

    #[test]
    fn normalise_collapses_separators() {
        assert_eq!(normalise("Foo_Bar-Baz"), "foo-bar-baz");
        assert_eq!(normalise("--foo--"), "foo");
    }
}
