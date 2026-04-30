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
    let idx = filter_actions_by_prompt("create a PR from my feature branch to main", &actions, 5);
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
