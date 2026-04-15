//! Fuzzy tool-filter for sub-agent delegation.
//!
//! When `skills_agent` is spawned with a bound Composio toolkit (e.g.
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
mod tests {
    use super::*;

    fn tool(name: &str, desc: &str) -> ConnectedIntegrationTool {
        ConnectedIntegrationTool {
            name: name.to_string(),
            description: desc.to_string(),
            parameters: None,
        }
    }

    fn github_sample() -> Vec<ConnectedIntegrationTool> {
        vec![
            tool("GITHUB_CREATE_A_PULL_REQUEST",
                 "Creates a pull request in a GitHub repository, requiring existing base and head branches."),
            tool("GITHUB_CREATE_A_REVIEW_FOR_A_PULL_REQUEST",
                 "Creates a pull request review, allowing approval, change requests, or comments."),
            tool("GITHUB_CREATE_A_DEPLOYMENT_BRANCH_POLICY",
                 "Creates a deployment branch or tag policy for an existing environment in a repository."),
            tool("GITHUB_DELETE_A_REVIEW_COMMENT_FOR_A_PULL_REQUEST",
                 "Deletes a review comment on a pull request."),
            tool("GITHUB_FIND_PULL_REQUESTS",
                 "Primary tool to find and search pull requests."),
            tool("GITHUB_GET_A_PULL_REQUEST",
                 "Retrieves a specific pull request by number."),
            tool("GITHUB_LIST_ASSIGNEES",
                 "Lists users who can be assigned to issues in a repository."),
        ]
    }

    #[test]
    fn create_pr_ranks_create_a_pull_request_first() {
        let actions = github_sample();
        let idx =
            filter_actions_by_prompt("create a PR from my feature branch to main", &actions, 5);
        assert!(!idx.is_empty());
        // Top match must be a CREATE verb tool (not DELETE/GET).
        let top_name = &actions[idx[0]].name;
        assert!(
            top_name.contains("CREATE") && top_name.contains("PULL_REQUEST"),
            "expected top match to be a CREATE + PULL_REQUEST tool, got {top_name}"
        );
        // The DELETE tool must not appear — verb gate should drop it.
        for &i in &idx {
            assert!(
                !actions[i].name.starts_with("GITHUB_DELETE"),
                "DELETE tool leaked past verb gate: {}",
                actions[i].name
            );
        }
    }

    #[test]
    fn list_prs_ranks_find_pull_requests_first() {
        let actions = github_sample();
        let idx = filter_actions_by_prompt("list open PRs assigned to me", &actions, 5);
        assert!(!idx.is_empty());
        let top_name = &actions[idx[0]].name;
        assert!(
            top_name == "GITHUB_FIND_PULL_REQUESTS" || top_name == "GITHUB_LIST_ASSIGNEES",
            "expected FIND_PULL_REQUESTS or LIST_ASSIGNEES on top, got {top_name}"
        );
    }

    #[test]
    fn empty_prompt_returns_empty() {
        let actions = github_sample();
        let idx = filter_actions_by_prompt("", &actions, 5);
        assert!(idx.is_empty());
    }

    #[test]
    fn abbreviation_expansion_works() {
        let qt = query_tokens("create a PR from feature branch");
        assert!(qt.contains("pr"));
        assert!(qt.contains("pull"));
        assert!(qt.contains("request"));
    }

    #[test]
    fn stopwords_removed() {
        let qt = query_tokens("send the email to my manager");
        assert!(!qt.contains("the"));
        assert!(!qt.contains("to"));
        assert!(!qt.contains("my"));
        assert!(qt.contains("send"));
        assert!(qt.contains("email"));
        assert!(qt.contains("manager"));
    }

    #[test]
    fn verb_detection_handles_aliases() {
        let v = detect_verbs("post a message to general channel");
        assert!(v.contains(&Verb::Send) || v.contains(&Verb::Create));

        let v = detect_verbs("delete all promotional emails");
        assert!(v.contains(&Verb::Delete));

        let v = detect_verbs("merge pull request 42");
        assert!(v.contains(&Verb::Merge));
    }

    #[test]
    fn tool_verb_handles_plurals() {
        assert_eq!(tool_verb("SLACK_DELETES_A_MESSAGE"), Some(Verb::Delete));
        assert_eq!(
            tool_verb("GITHUB_CREATE_A_PULL_REQUEST"),
            Some(Verb::Create)
        );
        assert_eq!(tool_verb("GMAIL_SEND_EMAIL"), Some(Verb::Send));
        assert_eq!(tool_verb("NOTION_QUERY_DATABASE"), Some(Verb::List));
        // Neutral — no verb prefix recognised
        assert_eq!(tool_verb("GITHUB_GIST_COMMENT"), None);
    }

    #[test]
    fn delete_query_excludes_create_tools() {
        let actions = vec![
            tool("GMAIL_SEND_EMAIL", "Sends an email."),
            tool("GMAIL_DELETE_MESSAGE", "Deletes a message by id."),
            tool("GMAIL_DELETE_THREAD", "Deletes a thread."),
            tool("GMAIL_BATCH_DELETE_MESSAGES", "Bulk delete messages."),
        ];
        let idx = filter_actions_by_prompt("delete all promotional emails", &actions, 10);
        for &i in &idx {
            assert!(
                actions[i].name.contains("DELETE"),
                "non-DELETE tool leaked: {}",
                actions[i].name
            );
        }
        assert!(idx.len() >= 3);
    }

    // ── Real-dataset integration tests ────────────────────────────────
    //
    // These run the filter against the actual Composio tool-list dump
    // for each toolkit (1000 tools total) captured from a live sidecar
    // `openhuman.composio_list_tools` call. Fixtures live in
    // `tests/fixtures/composio_<toolkit>.json`.

    fn load_real_toolkit(toolkit: &str) -> Vec<ConnectedIntegrationTool> {
        let path = format!(
            "{}/tests/fixtures/composio_{}.json",
            env!("CARGO_MANIFEST_DIR"),
            toolkit
        );
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture {path}: {e}"));
        let v: serde_json::Value =
            serde_json::from_str(&raw).unwrap_or_else(|e| panic!("failed to parse {path}: {e}"));
        let tools = v
            .pointer("/result/result/tools")
            .and_then(|t| t.as_array())
            .unwrap_or_else(|| panic!("missing /result/result/tools in {path}"));
        tools
            .iter()
            .map(|t| {
                let f = &t["function"];
                ConnectedIntegrationTool {
                    name: f["name"].as_str().unwrap_or("").to_string(),
                    description: f["description"].as_str().unwrap_or("").to_string(),
                    parameters: None,
                }
            })
            .collect()
    }

    /// Assert `wanted` shows up in the top-K indices of the filter output.
    fn assert_in_top(
        actions: &[ConnectedIntegrationTool],
        hits: &[usize],
        wanted: &str,
        label: &str,
    ) {
        let top_names: Vec<&str> = hits.iter().map(|&i| actions[i].name.as_str()).collect();
        assert!(
            top_names.iter().any(|n| *n == wanted),
            "[{label}] '{wanted}' not in top {k}: {top_names:?}",
            k = hits.len()
        );
    }

    #[test]
    fn real_data_github_create_pr() {
        let actions = load_real_toolkit("github");
        assert!(actions.len() > 400, "github fixture should have ~500 tools");
        let hits = filter_actions_by_prompt(
            "Create a pull request from feature/auth-fix to main in the openhuman repo",
            &actions,
            15,
        );
        assert!(hits.len() >= MIN_CONFIDENT_HITS);
        assert!(
            hits.len() < actions.len() / 5,
            "filter should narrow by >80%, got {}/{}",
            hits.len(),
            actions.len()
        );
        assert_in_top(
            &actions,
            &hits,
            "GITHUB_CREATE_A_PULL_REQUEST",
            "github create PR",
        );
    }

    #[test]
    fn real_data_github_list_prs() {
        let actions = load_real_toolkit("github");
        let hits = filter_actions_by_prompt(
            "Find all open pull requests assigned to the current user in the openhuman repo",
            &actions,
            15,
        );
        assert!(hits.len() >= MIN_CONFIDENT_HITS);
        assert_in_top(
            &actions,
            &hits,
            "GITHUB_FIND_PULL_REQUESTS",
            "github list PRs",
        );
    }

    #[test]
    fn real_data_gmail_send_email() {
        let actions = load_real_toolkit("gmail");
        let hits = filter_actions_by_prompt(
            "Send an email to john@example.com with subject 'Q2 Report' and body attached",
            &actions,
            10,
        );
        assert!(hits.len() >= MIN_CONFIDENT_HITS);
        assert_in_top(&actions, &hits, "GMAIL_SEND_EMAIL", "gmail send email");
        // Top 3 should all be send-related, not label/trash operations.
        for &i in hits.iter().take(3) {
            let n = &actions[i].name;
            assert!(
                n.contains("SEND") || n.contains("REPLY") || n.contains("DRAFT"),
                "non-send tool in top 3: {n}"
            );
        }
    }

    #[test]
    fn real_data_gmail_delete_emails() {
        let actions = load_real_toolkit("gmail");
        let hits = filter_actions_by_prompt(
            "Delete all promotional emails received in the last week",
            &actions,
            10,
        );
        assert!(hits.len() >= MIN_CONFIDENT_HITS);
        // All top results must be DELETE-flavoured, not send/fetch.
        for &i in &hits {
            let n = &actions[i].name;
            assert!(
                n.contains("DELETE") || n.contains("TRASH") || n.contains("REMOVE"),
                "non-delete tool in delete query top-K: {n}"
            );
        }
    }

    #[test]
    fn real_data_slack_send_message() {
        let actions = load_real_toolkit("slack");
        let hits = filter_actions_by_prompt(
            "Post a message to the #general channel saying the deploy is complete",
            &actions,
            15,
        );
        assert!(hits.len() >= MIN_CONFIDENT_HITS);
        assert_in_top(&actions, &hits, "SLACK_SEND_MESSAGE", "slack send message");
    }

    #[test]
    fn real_data_notion_create_page() {
        let actions = load_real_toolkit("notion");
        let hits = filter_actions_by_prompt(
            "Create a new page in the Engineering workspace titled 'Sprint Plan'",
            &actions,
            15,
        );
        assert!(hits.len() >= MIN_CONFIDENT_HITS);
        assert_in_top(
            &actions,
            &hits,
            "NOTION_CREATE_NOTION_PAGE",
            "notion create page",
        );
    }

    #[test]
    fn real_data_full_funnel_report() {
        // Non-asserting report showing the reduction ratio across all toolkits
        // for a representative query. Prints to stderr; run with
        // `cargo test real_data_full_funnel_report -- --nocapture`.
        let cases: &[(&str, &str)] = &[
            ("gmail", "send an email to the team about the release"),
            (
                "github",
                "create a pull request from feature branch to main",
            ),
            ("slack", "post a message to the general channel"),
            ("notion", "create a new page in the engineering database"),
            (
                "googlesheets",
                "add a row with today's sales to the revenue sheet",
            ),
            ("googledrive", "upload a file to the shared design folder"),
            ("instagram", "schedule a post with this photo and caption"),
            ("reddit", "comment on the top post in r/rust"),
            ("facebook", "post a status update to my page"),
        ];
        let mut total_in = 0usize;
        let mut total_out = 0usize;
        for (tk, q) in cases {
            let actions = load_real_toolkit(tk);
            let hits = filter_actions_by_prompt(q, &actions, 15);
            let kept = if hits.len() >= MIN_CONFIDENT_HITS {
                hits.len()
            } else {
                actions.len() // fallback path
            };
            total_in += actions.len();
            total_out += kept;
            eprintln!(
                "{:13} {:4} → {:3}   ({:5.1}% kept)   query: {}",
                tk,
                actions.len(),
                kept,
                100.0 * kept as f64 / actions.len() as f64,
                q
            );
        }
        eprintln!(
            "TOTAL         {total_in:4} → {total_out:3}   ({:5.1}% kept)",
            100.0 * total_out as f64 / total_in as f64
        );
        assert!(total_out < total_in / 3, "overall reduction should be >66%");
    }
}
