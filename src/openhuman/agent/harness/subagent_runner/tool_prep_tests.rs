use super::*;

#[test]
fn custom_delegate_is_treated_as_spawn_tool() {
    assert!(is_subagent_spawn_tool("spawn_subagent"));
    assert!(is_subagent_spawn_tool("delegate_researcher"));
    // Context scouting is top-level only — never visible to sub-agents
    // (incl. wildcard agents), which would otherwise scout the wrong
    // parent context. See #3949 review.
    assert!(is_subagent_spawn_tool("agent_prepare_context"));
    assert!(!is_subagent_spawn_tool("directory_resolve"));
}

#[test]
fn unprefixed_delegate_name_overrides_are_treated_as_spawn_tools() {
    // Most synthesised delegation tools use an unprefixed
    // `delegate_name` override (`plan`, `run_code`, `research`, …).
    // They must be stripped from every sub-agent surface, exactly like
    // the `delegate_*`-prefixed defaults.
    let tmp = tempfile::TempDir::new().unwrap();
    crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::init_global(tmp.path())
        .unwrap();
    for delegate in [
        "plan",
        "run_code",
        "research",
        "review_code",
        "do_crypto",
        "schedule_task",
        // `make_presentation` is `presentation_agent`'s `delegate_name`; the agent —
        // and therefore this delegate tool — is compiled out with the
        // `documents` feature.
        #[cfg(feature = "documents")]
        "make_presentation",
        "archive_session",
        // `use_mcp_server` is `mcp_agent`'s `delegate_name`; the agent —
        // and therefore this delegate tool — is compiled out with the
        // `mcp` feature (#4799). `setup_mcp_server` belongs to
        // `mcp_setup`, which stays registered in both builds.
        #[cfg(feature = "mcp")]
        "use_mcp_server",
        "setup_mcp_server",
    ] {
        assert!(
            is_subagent_spawn_tool(delegate),
            "`{delegate}` is a synthesised delegation tool and must be \
             stripped from sub-agent tool surfaces"
        );
    }
    // Ordinary worker tools stay visible.
    for plain in ["shell", "file_read", "web_fetch", "todo"] {
        assert!(
            !is_subagent_spawn_tool(plain),
            "`{plain}` must not be classified as a spawn tool"
        );
    }
}

// ── Essential-action reservation (#6033) ────────────────────────────────

use crate::openhuman::agent::context::prompt::ConnectedIntegrationTool;

fn action(name: &str) -> ConnectedIntegrationTool {
    ConnectedIntegrationTool {
        name: name.to_string(),
        description: format!("{name} description"),
        parameters: None,
    }
}

/// The catalogue the ranker sees for Gmail in the failing case: every
/// content-returning action gated out, twelve `LIST_*` slots kept.
fn gmail_catalogue() -> Vec<ConnectedIntegrationTool> {
    [
        "GMAIL_LIST_CSE_IDENTITIES",
        "GMAIL_LIST_DRAFTS",
        "GMAIL_LIST_LABELS",
        "GMAIL_LIST_MESSAGES",
        "GMAIL_LIST_THREADS",
        "GMAIL_FETCH_EMAILS",
        "GMAIL_FETCH_MESSAGE_BY_MESSAGE_ID",
        "GMAIL_SEND_EMAIL",
    ]
    .iter()
    .map(|n| action(n))
    .collect()
}

#[test]
fn essentials_are_reserved_ahead_of_ranked_hits() {
    let actions = gmail_catalogue();
    // The ranker picked only LIST_* actions — the #6033 repro.
    let hits = vec![0, 1, 2, 3, 4];
    let selected = select_actions_with_essentials("gmail", &actions, &hits, 12);
    let names: Vec<&str> = selected.iter().map(|&i| actions[i].name.as_str()).collect();

    assert_eq!(
        &names[..2],
        &["GMAIL_FETCH_EMAILS", "GMAIL_FETCH_MESSAGE_BY_MESSAGE_ID"],
        "essentials come first, in table order"
    );
    assert!(
        !names.contains(&"GMAIL_SEND_EMAIL"),
        "a read prompt must not be handed the send action: {names:?}"
    );
    assert!(
        names.contains(&"GMAIL_LIST_MESSAGES"),
        "ranked hits still follow the essentials"
    );
}

#[test]
fn an_essential_the_ranker_already_picked_costs_no_extra_slot() {
    let actions = gmail_catalogue();
    // Index 5 is GMAIL_FETCH_EMAILS — already ranked.
    let hits = vec![5, 0, 1];
    let selected = select_actions_with_essentials("gmail", &actions, &hits, 12);

    let fetches = selected
        .iter()
        .filter(|&&i| actions[i].name == "GMAIL_FETCH_EMAILS")
        .count();
    assert_eq!(
        fetches, 1,
        "no duplicate index for an already-ranked essential"
    );
    assert_eq!(
        selected.len(),
        selected
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        "selection is duplicate-free"
    );
}

#[test]
fn selection_never_exceeds_the_top_k_budget() {
    let actions = gmail_catalogue();
    let hits: Vec<usize> = (0..actions.len()).collect();
    for top_k in [0_usize, 1, 3, 4, 12] {
        let selected = select_actions_with_essentials("gmail", &actions, &hits, top_k);
        assert!(
            selected.len() <= top_k,
            "top_k={top_k} budget exceeded: {} kept",
            selected.len()
        );
        assert_eq!(
            selected.len(),
            selected
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            "top_k={top_k} produced a duplicate index"
        );
    }
}

#[test]
fn an_unlisted_toolkit_is_an_exact_passthrough() {
    let actions: Vec<_> = ["SLACK_SEND_MESSAGE", "SLACK_LIST_CHANNELS"]
        .iter()
        .map(|n| action(n))
        .collect();
    let hits = vec![1, 0];
    assert_eq!(
        select_actions_with_essentials("slack", &actions, &hits, 12),
        hits,
        "a toolkit with no essentials must rank exactly as before"
    );
}

#[test]
fn an_essential_absent_from_the_toolkit_is_skipped_not_guessed() {
    // Connected Gmail exposing none of the essentials: the helper must not
    // invent an index or panic.
    let actions: Vec<_> = ["GMAIL_LIST_LABELS", "GMAIL_LIST_THREADS"]
        .iter()
        .map(|n| action(n))
        .collect();
    let hits = vec![0, 1];
    let selected = select_actions_with_essentials("gmail", &actions, &hits, 12);
    assert_eq!(selected, hits);
    assert!(selected.iter().all(|&i| i < actions.len()));
}

#[test]
fn toolkit_lookup_is_case_insensitive() {
    assert_eq!(
        essential_actions_for_toolkit("Gmail"),
        essential_actions_for_toolkit("gmail")
    );
    assert!(!essential_actions_for_toolkit("GMAIL").is_empty());
    assert!(essential_actions_for_toolkit("slack").is_empty());
}

#[test]
fn every_essential_name_exists_in_the_real_toolkit_fixture() {
    // A stale table entry would silently spend no slot and hide the bug it
    // was added for, so pin the names against the real Composio dump.
    for (toolkit, essentials) in TOOLKIT_ESSENTIAL_ACTIONS {
        let path = format!(
            "{}/tests/fixtures/composio_{}.json",
            env!("CARGO_MANIFEST_DIR"),
            toolkit
        );
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture {path}: {e}"));
        let parsed: serde_json::Value =
            serde_json::from_str(&raw).unwrap_or_else(|e| panic!("failed to parse {path}: {e}"));
        let names: Vec<String> = parsed
            .pointer("/result/result/tools")
            .and_then(|t| t.as_array())
            .unwrap_or_else(|| panic!("missing /result/result/tools in {path}"))
            .iter()
            .filter_map(|t| {
                t.pointer("/function/name")
                    .and_then(|n| n.as_str())
                    .map(str::to_string)
            })
            .collect();
        for essential in *essentials {
            assert!(
                names.iter().any(|n| n == essential),
                "{toolkit} essential {essential} is not a real action in {path}"
            );
        }
    }
}

#[test]
fn gmail_read_prompt_keeps_a_content_returning_action() {
    // The two prompts from #6033: the ranker returns twelve LIST_* actions
    // and nothing that returns a message body. After reservation the
    // sub-agent can actually read an email.
    let path = format!(
        "{}/tests/fixtures/composio_gmail.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let raw = std::fs::read_to_string(&path).expect("gmail fixture");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("gmail fixture parses");
    let actions: Vec<ConnectedIntegrationTool> = parsed
        .pointer("/result/result/tools")
        .and_then(|t| t.as_array())
        .expect("fixture tools")
        .iter()
        .map(|t| ConnectedIntegrationTool {
            name: t
                .pointer("/function/name")
                .and_then(|n| n.as_str())
                .unwrap_or_default()
                .to_string(),
            description: t
                .pointer("/function/description")
                .and_then(|d| d.as_str())
                .unwrap_or_default()
                .to_string(),
            parameters: t.pointer("/function/parameters").cloned(),
        })
        .collect();

    for prompt in [
        "find job opportunities from the last 5 days",
        "Search the user's Gmail inbox for job opportunity emails from the last 5 days and summarize sender, subject and date",
    ] {
        let hits = crate::openhuman::agent::harness::tool_filter::filter_actions_by_prompt(
            prompt, &actions, 12,
        );
        let selected = select_actions_with_essentials("gmail", &actions, &hits, 12);
        let names: Vec<&str> = selected.iter().map(|&i| actions[i].name.as_str()).collect();

        assert!(
            names.contains(&"GMAIL_FETCH_EMAILS"),
            "prompt {prompt:?} must keep a content-returning action; got {names:?}"
        );
        assert!(selected.len() <= 12, "prompt {prompt:?} exceeded the budget");
        assert_eq!(
            selected.len(),
            selected
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            "prompt {prompt:?} produced a duplicate index"
        );
    }
}

#[test]
fn every_essential_action_is_read_only() {
    // Essentials are forced onto a delegation whatever it asked for, so a
    // write action here would hand a read task an unrequested
    // side-effecting capability that the approval middleware does not gate
    // (a ComposioActionTool does not override `external_effect`).
    const WRITE_VERBS: &[&str] = &[
        "SEND", "CREATE", "DELETE", "UPDATE", "PATCH", "MOVE", "MODIFY", "ADD", "REMOVE", "TRASH",
        "REPLY", "FORWARD", "DRAFT",
    ];
    for (toolkit, essentials) in TOOLKIT_ESSENTIAL_ACTIONS {
        for essential in *essentials {
            let verb = essential
                .split('_')
                .nth(1)
                .unwrap_or_default()
                .to_ascii_uppercase();
            assert!(
                !WRITE_VERBS.contains(&verb.as_str()),
                "{toolkit} essential {essential} is a write action; essentials must be read-only"
            );
        }
    }
}

// ── Dynamic-tool spawn strip (#6157) ────────────────────────────────────

use crate::openhuman::tools::Tool;
use async_trait::async_trait;

/// A tool that is nothing but its name — the strip reads no other field.
struct NamedTool(&'static str);

#[async_trait]
impl Tool for NamedTool {
    /// The caller-chosen name.
    fn name(&self) -> &str {
        self.0
    }

    /// Unused by the strip.
    fn description(&self) -> &str {
        "stub"
    }

    /// Unused by the strip.
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({})
    }

    /// Never called: the strip only inspects names.
    async fn execute(
        &self,
        _args: serde_json::Value,
    ) -> anyhow::Result<crate::openhuman::tools::traits::ToolResult> {
        Ok(crate::openhuman::tools::traits::ToolResult::success(
            String::new(),
        ))
    }
}

/// A dynamic-tool list carrying exactly these names, in this order.
fn dynamic(names: &[&'static str]) -> Vec<Box<dyn Tool>> {
    names
        .iter()
        .map(|name| Box::new(NamedTool(name)) as Box<dyn Tool>)
        .collect()
}

/// The surviving names, so a test can assert on content *and* order.
fn names_of(tools: &[Box<dyn Tool>]) -> Vec<&str> {
    tools.iter().map(|t| t.name()).collect()
}

/// The shape the runner actually builds: Composio actions plus the
/// progressive-disclosure handoff tool, with spawn tools interleaved.
#[test]
fn dynamic_tools_keep_ordinary_actions_and_lose_spawn_tools() {
    let mut tools = dynamic(&[
        "GMAIL_FETCH_EMAILS",
        "spawn_subagent",
        "extract_from_result",
        "delegate_graph",
        "spawn_worker_thread",
        "GMAIL_SEND_EMAIL",
        "agent_prepare_context",
    ]);
    strip_spawn_tools_from_dynamic(&mut tools, "researcher");

    assert_eq!(
        names_of(&tools),
        vec![
            "GMAIL_FETCH_EMAILS",
            "extract_from_result",
            "GMAIL_SEND_EMAIL"
        ],
        "only spawn/delegate tools are dropped, and surviving order is preserved"
    );
}

/// The whole reason the strip lives here rather than relying on the
/// registration-time backstop: that backstop matches `delegate_*` by prefix and
/// would keep `plan`, which is `planner`'s `delegate_name`.
#[test]
fn dynamic_tools_lose_unprefixed_delegate_name_overrides() {
    let tmp = tempfile::TempDir::new().unwrap();
    crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::init_global(tmp.path())
        .unwrap();

    let mut tools = dynamic(&["plan", "run_code", "shell", "web_fetch"]);
    strip_spawn_tools_from_dynamic(&mut tools, "researcher");

    assert_eq!(
        names_of(&tools),
        vec!["shell", "web_fetch"],
        "`delegate_name` overrides are spawn tools even without the prefix"
    );
}

/// The live case today: no dynamic tool is named after a spawn tool, so the
/// strip must be a no-op rather than a silent narrowing of the surface.
#[test]
fn a_dynamic_tool_list_without_spawn_tools_is_untouched() {
    let mut tools = dynamic(&["GMAIL_FETCH_EMAILS", "extract_from_result"]);
    strip_spawn_tools_from_dynamic(&mut tools, "researcher");

    assert_eq!(
        names_of(&tools),
        vec!["GMAIL_FETCH_EMAILS", "extract_from_result"]
    );
}
