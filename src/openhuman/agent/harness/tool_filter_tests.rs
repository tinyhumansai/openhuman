//! Host-side coverage for the tool-filter adapter.
//!
//! The ranking algorithm itself is tested upstream in
//! `tinyagents_harness::tool::select`, against synthetic catalogues. What is
//! tested here is the half that stayed: the `ConnectedIntegrationTool`
//! adapter, exercised against the real Composio tool-list dumps in
//! `tests/fixtures/` (1000 actions across nine toolkits). Those fixtures are
//! host data about a specific integration provider and have no business in a
//! provider-neutral library crate, so this suite stays here rather than
//! moving up with the algorithm.

use super::*;
use crate::openhuman::agent::context::prompt::ConnectedIntegrationTool;

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
fn assert_in_top(actions: &[ConnectedIntegrationTool], hits: &[usize], wanted: &str, label: &str) {
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

// ── Repro: issue #3152 — Composio write action unreachable ──────────
//
// `integrations_agent` asked to CREATE a Notion page. Notion is a
// HEAVY_SCHEMA toolkit → production top_k = 12. The verb gate + score cull
// advertise only read-leaning actions, so `NOTION_CREATE_NOTION_PAGE` never
// reaches the model. Asserts DESIRED post-fix behaviour → RED until the
// write-reservation fix lands.
#[test]
fn repro_3152_create_page_reachable_in_top_k() {
    let actions = load_real_toolkit("notion");
    let hits = filter_actions_by_prompt(
        "create a notion page with the meeting notes and give me the link",
        &actions,
        12,
    );
    assert_in_top(
        &actions,
        &hits,
        "NOTION_CREATE_NOTION_PAGE",
        "#3152 notion create-page",
    );
}

// ── Ranking-drift guard ────────────────────────────────────────────
//
// Exact top-5 orderings captured from the pre-extraction implementation,
// before the algorithm moved into `tinyagents`. The ranker decides which
// actions a model is shown, so a scoring change is a silent behaviour change;
// these pin the ordering through the adapter, over real catalogues.
#[test]
fn adapter_ranking_matches_the_pre_extraction_snapshot() {
    let cases: &[(&str, &str, usize, &[&str])] = &[
        (
            "github",
            "Create a pull request from feature/auth-fix to main in the openhuman repo",
            15,
            &[
                "GITHUB_CREATE_A_CODESPACE_FROM_A_PULL_REQUEST",
                "GITHUB_CREATE_A_PULL_REQUEST",
                "GITHUB_CREATE_REACTION_FOR_A_PULL_REQUEST_REVIEW_COMMENT",
                "GITHUB_CREATE_A_REVIEW_COMMENT_FOR_A_PULL_REQUEST",
                "GITHUB_CREATE_A_REVIEW_FOR_A_PULL_REQUEST",
            ],
        ),
        (
            "gmail",
            "Send an email to john@example.com with subject 'Q2 Report' and body attached",
            10,
            &[
                "GMAIL_SEND_EMAIL",
                "GMAIL_SEND_DRAFT",
                "GMAIL_SETTINGS_SEND_AS_GET",
                "GMAIL_REPLY_TO_THREAD",
                "GMAIL_FORWARD_MESSAGE",
            ],
        ),
        (
            "slack",
            "Post a message to the #general channel saying the deploy is complete",
            15,
            &[
                "SLACK_SEND_MESSAGE",
                "SLACK_SEND_EPHEMERAL_MESSAGE",
                "SLACK_SEND_ME_MESSAGE",
                "SLACK_CREATE_CHANNEL",
                "SLACK_CREATE_CHANNEL_BASED_CONVERSATION",
            ],
        ),
        (
            "notion",
            "Create a new page in the Engineering workspace titled 'Sprint Plan'",
            15,
            &[
                "NOTION_CREATE_NOTION_PAGE",
                "NOTION_CREATE_COMMENT",
                "NOTION_ADD_MULTIPLE_PAGE_CONTENT",
                "NOTION_ADD_PAGE_CONTENT",
                "NOTION_CREATE_DATABASE",
            ],
        ),
    ];
    for (toolkit, prompt, top_k, expected) in cases {
        let actions = load_real_toolkit(toolkit);
        let got: Vec<&str> = filter_actions_by_prompt(prompt, &actions, *top_k)
            .into_iter()
            .take(expected.len())
            .map(|i| actions[i].name.as_str())
            .collect();
        assert_eq!(
            &got, expected,
            "ranking drifted for {toolkit} prompt {prompt:?}"
        );
    }
}
