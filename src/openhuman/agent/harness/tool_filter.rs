//! Fuzzy tool-filter for sub-agent delegation.
//!
//! When `integrations_agent` is spawned with a bound Composio toolkit (e.g.
//! `toolkit="github"`), the parent-refined task prompt is usually specific
//! enough that only a handful of the toolkit's actions are relevant. Github's
//! catalogue alone has 500 actions; loading every one into the sub-agent's
//! tool set balloons prompt size and confuses the model.
//!
//! This module ranks the actions against the task prompt using a cheap
//! five-stage pipeline — no model load, pure CPU, stdlib only:
//!
//! 1. **Verb detection** — map the prompt to CRUD-ish intents
//!    (`create`/`send`/`read`/`list`/`update`/`delete`/`merge`).
//! 2. **Verb gate** — drop actions whose first-word verb conflicts with
//!    the detected intent. Tools with a neutral prefix (e.g. `GITHUB_FIND_*`)
//!    are kept as ambiguous.
//! 3. **Query token expansion** — strip stopwords, expand common
//!    abbreviations (`pr` → `pull request`, `dm` → `direct message`) so
//!    the ranker can match the user's casual phrasing against the
//!    toolkit's formal action names.
//! 4. **Weighted token overlap** — 3× weight on hits in the action name,
//!    1× on hits in the description. Cheap, effective, explainable.
//! 5. **Verb-alignment boost** — small additive bonus when the action's
//!    first-word verb matches the detected intent, penalty when it
//!    clearly conflicts.
//!
//! Entry point: [`filter_actions_by_prompt`].

use std::collections::HashSet;

use crate::openhuman::context::prompt::ConnectedIntegrationTool;

/// Minimum number of hits the filter must produce to be trusted. Below this,
/// the caller should fall back to the unfiltered toolkit — a too-narrow filter
/// is worse than no filter at all because it starves the sub-agent.
pub const MIN_CONFIDENT_HITS: usize = 3;

/// Rank `actions` against `prompt` and return indices for the top
/// `max_results` matches, ordered best-first.
///
/// Returns an empty `Vec` when `prompt` is empty or no token hits are found —
/// callers should check `.len() < MIN_CONFIDENT_HITS` and fall back to the
/// unfiltered toolkit in that case.
pub fn filter_actions_by_prompt(
    prompt: &str,
    actions: &[ConnectedIntegrationTool],
    max_results: usize,
) -> Vec<usize> {
    if prompt.trim().is_empty() || actions.is_empty() {
        return Vec::new();
    }

    let verbs = detect_verbs(prompt);
    let qt = query_tokens(prompt);

    // Stage 1-2: verb gate. Keep actions whose verb matches the query,
    // or whose prefix is neutral (no recognised verb).
    let gated: Vec<usize> = actions
        .iter()
        .enumerate()
        .filter(|(_, a)| {
            if verbs.is_empty() {
                return true;
            }
            match tool_verb(&a.name) {
                Some(v) => verbs.contains(&v),
                None => true,
            }
        })
        .map(|(i, _)| i)
        .collect();

    // Stage 3-5: weighted token overlap + verb-alignment bonus, then sort.
    let mut scored: Vec<(i32, usize)> = gated
        .iter()
        .map(|&i| {
            let a = &actions[i];
            let score =
                weighted_overlap(&qt, &a.name, &a.description) + verb_bonus(&a.name, &verbs);
            (score, i)
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0));

    // Only keep positively-scored results. Zero-overlap tools would add noise.
    scored
        .into_iter()
        .filter(|(s, _)| *s > 0)
        .take(max_results)
        .map(|(_, i)| i)
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────
// Verb detection
// ─────────────────────────────────────────────────────────────────────────

/// Detected query intent. A small, stable set — expanding it risks
/// over-matching (e.g. "open" is deliberately excluded because it appears in
/// both "open a PR" and "open PRs").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verb {
    Create,
    Send,
    Read,
    List,
    Update,
    Delete,
    Merge,
}

fn verb_aliases(v: Verb) -> &'static [&'static str] {
    match v {
        Verb::Create => &[
            "create", "make", "new", "add", "start", "write", "post", "draft",
        ],
        Verb::Send => &[
            "send", "email", "message", "dm", "reply", "forward", "notify",
        ],
        Verb::Read => &["read", "get", "fetch", "show", "view", "see", "retrieve"],
        Verb::List => &["list", "search", "find", "lookup", "browse"],
        Verb::Update => &[
            "update", "edit", "modify", "change", "rename", "move", "set",
        ],
        Verb::Delete => &["delete", "remove", "drop", "archive", "unsubscribe"],
        Verb::Merge => &["merge", "accept", "approve"],
    }
}

const ALL_VERBS: [Verb; 7] = [
    Verb::Create,
    Verb::Send,
    Verb::Read,
    Verb::List,
    Verb::Update,
    Verb::Delete,
    Verb::Merge,
];

/// Tool-name prefixes (uppercase, after the toolkit prefix is stripped)
/// that map to each verb. Checked against the first two words of the
/// stripped tool name; trailing `S` is tolerated (`DELETES` → `DELETE`).
fn tool_verb_prefixes(v: Verb) -> &'static [&'static str] {
    match v {
        Verb::Create => &["CREATE", "ADD", "NEW", "POST", "DRAFT", "START", "INSERT"],
        Verb::Send => &["SEND", "REPLY", "FORWARD", "NOTIFY"],
        Verb::Read => &[
            "GET", "FETCH", "SHOW", "READ", "RETRIEVE", "DESCRIBE", "CHECK",
        ],
        Verb::List => &["LIST", "SEARCH", "FIND", "BROWSE", "COUNT", "QUERY"],
        Verb::Update => &[
            "UPDATE", "EDIT", "MODIFY", "RENAME", "MOVE", "SET", "PATCH", "UPSERT",
        ],
        Verb::Delete => &["DELETE", "REMOVE", "DROP", "ARCHIVE", "UNSUBSCRIBE"],
        Verb::Merge => &["MERGE", "APPROVE", "ACCEPT", "DISMISS"],
    }
}

fn detect_verbs(prompt: &str) -> HashSet<Verb> {
    let lowered = prompt.to_ascii_lowercase();
    let mut found = HashSet::new();
    for &v in &ALL_VERBS {
        for alias in verb_aliases(v) {
            if contains_whole_word(&lowered, alias) {
                found.insert(v);
                break;
            }
        }
    }
    found
}

/// Classify a tool name (e.g. `"GITHUB_CREATE_A_PULL_REQUEST"`) by verb.
/// Returns `None` when no verb prefix is recognised — such tools are kept as
/// neutral by the gate.
fn tool_verb(name: &str) -> Option<Verb> {
    // Strip the toolkit prefix (everything up to and including the first `_`).
    let stripped = match name.split_once('_') {
        Some((_, rest)) => rest,
        None => name,
    };
    // Check the first two words.
    for word in stripped.split('_').take(2) {
        let trimmed = word.strip_suffix('S').unwrap_or(word);
        for &v in &ALL_VERBS {
            for &prefix in tool_verb_prefixes(v) {
                if word == prefix || trimmed == prefix {
                    return Some(v);
                }
            }
        }
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────
// Token handling
// ─────────────────────────────────────────────────────────────────────────

const STOPWORDS: &[&str] = &[
    "the", "a", "an", "to", "from", "for", "of", "with", "my", "me", "i", "and", "or", "on", "in",
    "at", "is", "are", "by", "this", "that", "it", "about", "all", "any", "some", "new", "old",
];

/// Bidirectional abbreviation map applied to query tokens. If the query has
/// `pr`, we add `pull` and `request`; if the tool name has `PULL_REQUEST` and
/// the query has `pr`, this bridges them.
const ABBREVS: &[(&str, &[&str])] = &[
    ("pr", &["pull", "request"]),
    ("prs", &["pull", "requests"]),
    ("dm", &["direct", "message"]),
    ("dms", &["direct", "messages"]),
    ("repo", &["repository"]),
    ("repos", &["repositories"]),
    ("org", &["organization"]),
    ("orgs", &["organizations"]),
    ("msg", &["message"]),
    ("ch", &["channel"]),
];

/// Tokenize a string into lowercase alphanumeric words.
fn tokenize(s: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut current = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            current.push(c.to_ascii_lowercase());
        } else if !current.is_empty() {
            out.insert(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.insert(current);
    }
    out
}

fn query_tokens(query: &str) -> HashSet<String> {
    let raw: HashSet<String> = tokenize(query)
        .into_iter()
        .filter(|t| t.len() > 1 && !STOPWORDS.contains(&t.as_str()))
        .collect();
    let mut expanded = raw.clone();
    for t in &raw {
        for (abbr, replacements) in ABBREVS {
            if t == abbr {
                for r in *replacements {
                    expanded.insert((*r).to_string());
                }
            }
        }
    }
    expanded
}

fn weighted_overlap(qt: &HashSet<String>, name: &str, desc: &str) -> i32 {
    let name_tokens = tokenize(name);
    let desc_tokens = tokenize(desc);
    let name_hits = qt.intersection(&name_tokens).count() as i32;
    let desc_hits = qt.intersection(&desc_tokens).count() as i32;
    3 * name_hits + desc_hits
}

fn verb_bonus(name: &str, query_verbs: &HashSet<Verb>) -> i32 {
    if query_verbs.is_empty() {
        return 0;
    }
    match tool_verb(name) {
        Some(v) if query_verbs.contains(&v) => 3,
        Some(_) => -2,
        None => 0,
    }
}

fn contains_whole_word(haystack: &str, needle: &str) -> bool {
    // Cheap whole-word check without regex. Works on ASCII; prompts from
    // orchestrators are essentially ASCII anyway.
    let mut start = 0;
    while let Some(idx) = haystack[start..].find(needle) {
        let abs = start + idx;
        let before_ok = abs == 0 || !haystack.as_bytes()[abs - 1].is_ascii_alphanumeric();
        let end = abs + needle.len();
        let after_ok = end == haystack.len() || !haystack.as_bytes()[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        start = abs + 1;
    }
    false
}

// ─────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "tool_filter_tests.rs"]
mod tests;
