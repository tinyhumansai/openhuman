use super::*;
use async_trait::async_trait;
use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;
use tinytools::Tool;

static NO_FILTER: LazyLock<HashSet<String>> = LazyLock::new(HashSet::new);

struct TestTool;

#[async_trait]
impl Tool for TestTool {
    fn name(&self) -> &str {
        "test_tool"
    }

    fn description(&self) -> &str {
        "tool desc"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<tinytools::ToolResult> {
        Ok(tinytools::ToolResult::success("ok"))
    }
}

fn ctx_with_identity(identity: Option<UserIdentity>) -> PromptContext<'static> {
    use std::sync::OnceLock;
    static EMPTY_VISIBLE: OnceLock<HashSet<String>> = OnceLock::new();
    let visible = EMPTY_VISIBLE.get_or_init(HashSet::new);
    static EMPTY_TOOLS: &[PromptTool<'static>] = &[];
    static EMPTY_INTEGRATIONS: &[ConnectedIntegration] = &[];
    PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: EMPTY_TOOLS,
        workflows: &[],
        dispatcher_instructions: "",
        visible_tool_names: visible,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: EMPTY_INTEGRATIONS,
        connected_identities_md: String::new(),
        user_identity: identity,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AGENTS.md project-instructions section
// ─────────────────────────────────────────────────────────────────────────────

/// Build a minimal `PromptContext` carrying the given AGENTS.md layers.
/// Everything else is inert so tests isolate the AGENTS.md behaviour.
fn agents_md_ctx(global: Option<String>, local: Option<String>) -> PromptContext<'static> {
    PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: &[],
        workflows: &[],
        dispatcher_instructions: "",
        visible_tool_names: &NO_FILTER,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        user_identity: None,
        personality_roster: vec![],
        agents_md_global: global,
        agents_md_local: local,
    }
}

#[path = "mod_tests_agents_md_registration_tests.rs"]
mod agents_md_registration_tests;
#[path = "mod_tests_builder_sections_tests.rs"]
mod builder_sections_tests;
#[path = "mod_tests_subagent_render_tests.rs"]
mod subagent_render_tests;
#[path = "mod_tests_tools_sections_tests.rs"]
mod tools_sections_tests;

#[test]
fn tool_call_format_maps_to_the_dialect_and_harness_vocabulary() {
    use tinyagents_harness::config::ToolDispatcher;
    use tinytools_agent::dialect::CodeStyle;

    for (dialect, host, harness, style) in [
        (
            tinytools_agent::dialect::ToolCallFormat::PFormat,
            ToolCallFormat::PFormat,
            ToolDispatcher::Pformat,
            None,
        ),
        (
            tinytools_agent::dialect::ToolCallFormat::Json,
            ToolCallFormat::Json,
            ToolDispatcher::Xml,
            None,
        ),
        // Native maps to Auto on purpose: the harness keeps its profile-driven
        // fallback for a model that turns out not to support native tools.
        (
            tinytools_agent::dialect::ToolCallFormat::Native,
            ToolCallFormat::Native,
            ToolDispatcher::Auto,
            None,
        ),
        (
            tinytools_agent::dialect::ToolCallFormat::Python,
            ToolCallFormat::Python,
            ToolDispatcher::Python,
            Some(CodeStyle::Python),
        ),
        (
            tinytools_agent::dialect::ToolCallFormat::TypeScript,
            ToolCallFormat::TypeScript,
            ToolDispatcher::Typescript,
            Some(CodeStyle::TypeScript),
        ),
    ] {
        assert_eq!(tool_call_format_from_dialect(dialect), host);
        assert_eq!(host.harness_dispatcher(), harness);
        assert_eq!(host.code_style(), style);
    }
}

/// The prompt catalogue is the callable surface on a text dialect, so a
/// deferred tool must leave it and the discovery bridge (`tool_search`) must
/// take its place.
/// Rendering the deferred set instead (what the policy allow-set does, since
/// it admits those names to keep a found tool callable) both spends the bytes
/// deferral exists to save and tells the model to search for a signature it
/// can already read.
#[test]
fn swapping_deferred_entries_leaves_the_bridge_in_their_place() {
    let mut tools = vec![
        PromptTool::new("shell", "Run a command."),
        PromptTool::new("GMAIL_SEND_EMAIL", "Send an email."),
        PromptTool::new("GMAIL_FETCH_EMAILS", "Read email."),
    ];
    let mut visible: HashSet<String> = ["shell", "GMAIL_SEND_EMAIL", "GMAIL_FETCH_EMAILS"]
        .into_iter()
        .map(str::to_string)
        .collect();
    let deferred: HashSet<String> = ["GMAIL_SEND_EMAIL", "GMAIL_FETCH_EMAILS"]
        .into_iter()
        .map(str::to_string)
        .collect();

    swap_deferred_for_discovery_bridge(&mut tools, &mut visible, &deferred);

    assert!(visible.contains("shell"), "a direct tool stays advertised");
    assert!(
        !visible.contains("GMAIL_SEND_EMAIL") && !visible.contains("GMAIL_FETCH_EMAILS"),
        "deferred tools must not be rendered into the catalogue: {visible:?}"
    );
    assert!(
        visible.contains("tool_search") && !visible.contains("tool_call"),
        "`tool_search` replaces them; there is no `tool_call` wrapper: {visible:?}"
    );
    {
        let name = "tool_search";
        assert!(
            tools.iter().any(|tool| tool.name == name
                && tool
                    .parameters_schema
                    .as_deref()
                    .is_some_and(|schema| schema.contains("\"type\":\"object\""))),
            "{name} must reach the catalogue with a callable schema"
        );
    }
}

/// Nothing deferred: the catalogue and the advertised set are untouched, and
/// a belt that never opted into discovery does not pay for a bridge
/// schema it cannot use.
#[test]
fn swapping_is_a_no_op_without_a_deferred_set() {
    let mut tools = vec![PromptTool::new("shell", "Run a command.")];
    let mut visible: HashSet<String> = ["shell"].into_iter().map(str::to_string).collect();

    swap_deferred_for_discovery_bridge(&mut tools, &mut visible, &HashSet::new());

    assert_eq!(tools.len(), 1);
    assert_eq!(visible.len(), 1);
    assert!(visible.contains("shell"));
}
