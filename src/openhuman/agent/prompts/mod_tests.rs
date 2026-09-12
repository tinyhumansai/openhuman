use super::*;
use crate::openhuman::tools::traits::Tool;
use async_trait::async_trait;
use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

static NO_FILTER: LazyLock<HashSet<String>> = LazyLock::new(HashSet::new);

/// Build a `NamespaceSummary` with a fixed `updated_at` (#2944), so
/// freshness-label assertions are deterministic.
fn ns_summary_at(namespace: &str, body: &str, rfc3339: &str) -> NamespaceSummary {
    NamespaceSummary {
        namespace: namespace.into(),
        body: body.into(),
        updated_at: chrono::DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .with_timezone(&chrono::Utc),
    }
}

/// `NamespaceSummary` with an arbitrary fixed date, for tests that don't
/// assert on the freshness stamp itself.
fn ns_summary(namespace: &str, body: &str) -> NamespaceSummary {
    ns_summary_at(namespace, body, "2026-01-01T00:00:00Z")
}

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

    async fn execute(
        &self,
        _args: serde_json::Value,
    ) -> anyhow::Result<crate::openhuman::tools::ToolResult> {
        Ok(crate::openhuman::tools::ToolResult::success("ok"))
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
        learned: LearnedContextData::default(),
        visible_tool_names: visible,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: EMPTY_INTEGRATIONS,
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
        user_identity: identity,
        personality_soul_md: None,
        personality_memory_md: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
    }
}

/// Shared `PromptContext` for the MEMORY.md-framing tests below. Both
/// exercise `UserFilesSection` with memory injection enabled and differ
/// only in workspace contents, so they build an identical 19-field
/// context — factor it out so the two can't drift when `PromptContext`
/// gains fields. Borrows the caller's `workspace` and pre-built
/// `prompt_tools` so the returned context outlives neither.
fn memory_framing_ctx<'a>(
    workspace: &'a std::path::Path,
    prompt_tools: &'a [PromptTool<'a>],
) -> PromptContext<'a> {
    PromptContext {
        workspace_dir: workspace,
        model_name: "test-model",
        agent_id: "",
        tools: prompt_tools,
        workflows: &[],
        dispatcher_instructions: "",
        learned: LearnedContextData::default(),
        visible_tool_names: &NO_FILTER,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: true,
        curated_snapshot: None,
        user_identity: None,
        personality_soul_md: None,
        personality_memory_md: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
    }
}

fn ctx_with_learned(learned: LearnedContextData) -> PromptContext<'static> {
    let prompt_tools: &'static [PromptTool<'static>] = &[];
    PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: prompt_tools,
        workflows: &[],
        dispatcher_instructions: "",
        learned,
        visible_tool_names: &NO_FILTER,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
        user_identity: None,
        personality_soul_md: None,
        personality_memory_md: None,
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
        learned: LearnedContextData::default(),
        visible_tool_names: &NO_FILTER,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
        user_identity: None,
        personality_soul_md: None,
        personality_memory_md: None,
        personality_roster: vec![],
        agents_md_global: global,
        agents_md_local: local,
    }
}

#[path = "mod_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "mod_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "mod_tests_part_03_tests.rs"]
mod part_03_tests;
#[path = "mod_tests_part_04_tests.rs"]
mod part_04_tests;
