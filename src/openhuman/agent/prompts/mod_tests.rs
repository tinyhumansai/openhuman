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

#[test]
fn prompt_builder_assembles_sections() {
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let prompt_tools = PromptTool::from_tools(&tools);
    let ctx = PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: &prompt_tools,
        workflows: &[],
        dispatcher_instructions: "instr",
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
    };
    let rendered = SystemPromptBuilder::with_defaults().build(&ctx).unwrap();
    assert!(rendered.contains("## Tools"));
    assert!(rendered.contains("test_tool"));
    assert!(rendered.contains("instr"));
}

#[test]
fn grounding_contract_appended_to_every_build_path() {
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let prompt_tools = PromptTool::from_tools(&tools);
    let ctx = PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: &prompt_tools,
        workflows: &[],
        dispatcher_instructions: "instr",
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
    };

    // A distinctive clause from GROUNDING_BODY — present regardless of which
    // builder produced the prompt (single source of truth, central append).
    let marker = "Your tools are exactly the ones listed in this prompt";

    // 1. Static default chain.
    let defaults = SystemPromptBuilder::with_defaults().build(&ctx).unwrap();
    assert!(defaults.contains("## Grounding and tool use"));
    assert!(defaults.contains(marker));

    // 2. Sub-agent static chain.
    let sub = SystemPromptBuilder::for_subagent("role".into(), true, true, true)
        .build(&ctx)
        .unwrap();
    assert!(sub.contains(marker));

    // 3. Dynamic builder (the path every `agents/<id>/prompt.rs` uses). The
    //    dynamic body itself does NOT contain grounding; the wrapping
    //    `build()` appends it, so all 26 dynamic agents inherit it for free.
    //    `PromptBuilder` is a bare `fn` pointer, so this must be a
    //    non-capturing fn item, not a closure.
    fn dynamic_body_builder(_ctx: &PromptContext<'_>) -> anyhow::Result<String> {
        Ok("## Custom Agent\n\nI render my own body.".to_string())
    }
    let dynamic = SystemPromptBuilder::from_dynamic(dynamic_body_builder)
        .build(&ctx)
        .unwrap();
    assert!(dynamic.contains("I render my own body."));
    assert!(dynamic.contains(marker));

    // 4. It is appended once, not duplicated.
    assert_eq!(
        defaults.matches("## Grounding and tool use").count(),
        1,
        "grounding contract must appear exactly once"
    );

    // Appears before the output-style suffix (tail placement).
    let g = defaults.find("## Grounding and tool use").unwrap();
    let s = defaults.find("## Output style").unwrap();
    assert!(g < s, "grounding should precede the output-style suffix");
}

#[test]
fn grounding_contract_requires_exact_numeric_evidence() {
    let ctx = ctx_with_identity(None);
    let rendered = SystemPromptBuilder::from_final_body("## Custom Agent\n\nBody.".into())
        .build(&ctx)
        .unwrap();

    assert!(rendered.contains("Preserve numeric evidence exactly"));
    assert!(rendered.contains(
        "numbers, counts, sizes, dates, timestamps, durations, currencies, percentages, quotas, and ids"
    ));
    assert!(rendered.contains(
        "copy the exact value from the observed tool result, user message, or cited memory"
    ));
    assert!(rendered.contains("Do not round, convert units, rewrite relative times"));
    assert!(rendered.contains(
        "If sources disagree, name the discrepancy instead of choosing a plausible value"
    ));
}

#[test]
fn identity_section_creates_missing_workspace_files() {
    let workspace =
        std::env::temp_dir().join(format!("openhuman_prompt_create_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workspace).unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![];
    let prompt_tools = PromptTool::from_tools(&tools);
    let ctx = PromptContext {
        workspace_dir: &workspace,
        model_name: "test-model",
        agent_id: "",
        tools: &prompt_tools,
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
    };

    let section = IdentitySection;
    let _ = section.build(&ctx).unwrap();

    for file in ["SOUL.md", "IDENTITY.md", "HEARTBEAT.md"] {
        assert!(
            workspace.join(file).exists(),
            "expected workspace file to be created: {file}"
        );
    }
    let soul = std::fs::read_to_string(workspace.join("SOUL.md")).unwrap();
    assert!(
        soul.starts_with("# OpenHuman"),
        "SOUL.md should be seeded from src/openhuman/agent/prompts/SOUL.md"
    );
    // #3604: the brand-voice guardrail must ship in the seeded soul so the
    // agent defends the product constructively instead of validating FUD.
    assert!(
        soul.contains("## When OpenHuman is criticized"),
        "SOUL.md must carry the brand-voice section (#3604)"
    );
    assert!(
        soul.contains("Don't validate FUD"),
        "SOUL.md brand-voice section must keep the do-not-validate-FUD directive (#3604)"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn datetime_section_is_static_grounding_rule_without_volatile_timestamp() {
    // #3602: the concrete "now" moved to the per-turn user message
    // (`current_datetime_line`) so a long-lived session's frozen
    // system-prompt prefix never goes stale. The section must therefore
    // carry the greeting/clock grounding *rule* but NOT a volatile
    // timestamp — otherwise the prefix is no longer byte-stable and a
    // stale clock contradicts the fresh per-turn one.
    let tools: Vec<Box<dyn Tool>> = vec![];
    let prompt_tools = PromptTool::from_tools(&tools);
    let ctx = PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: &prompt_tools,
        workflows: &[],
        dispatcher_instructions: "instr",
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
    };

    let rendered = DateTimeSection.build(&ctx).unwrap();
    assert!(rendered.starts_with("## Current Date & Time\n\n"));
    // Greeting/clock grounding rule must be present, ungated (no tools here).
    assert!(
        rendered.contains("good morning") && rendered.contains("match the actual local hour"),
        "datetime section must carry the greeting-grounding rule; got:\n{rendered}"
    );
    assert!(
        rendered.contains("Current Date & Time:"),
        "rule must point at the per-turn `Current Date & Time:` line; got:\n{rendered}"
    );
    // Byte-stability guard: two renders a moment apart must be identical —
    // i.e. no embedded volatile clock. A frozen timestamp would make these
    // diverge (and bust the KV-cache prefix).
    let again = DateTimeSection.build(&ctx).unwrap();
    assert_eq!(
        rendered, again,
        "datetime section must be byte-stable (no volatile timestamp baked in)"
    );
}

#[test]
fn current_datetime_line_is_fresh_local_stamp() {
    // The per-turn stamp carries a parseable local date, IANA zone (or the
    // `UTC` fallback), a UTC offset, and the weekday — everything the model
    // needs to localize a greeting without a tool call (#3602).
    let line = super::current_datetime_line();
    let rest = line
        .strip_prefix("Current Date & Time: ")
        .unwrap_or_else(|| panic!("stamp must start with canonical prefix: {line}"));
    // The first 19 chars must be a canonical `YYYY-MM-DD HH:MM:SS`.
    let dt = rest
        .get(0..19)
        .unwrap_or_else(|| panic!("stamp too short for YYYY-MM-DD HH:MM:SS: {line}"));
    chrono::NaiveDateTime::parse_from_str(dt, "%Y-%m-%d %H:%M:%S")
        .unwrap_or_else(|e| panic!("timestamp must match YYYY-MM-DD HH:MM:SS ({e}): {line}"));
    assert!(line.contains("UTC"), "missing UTC offset: {line}");
    assert!(
        line.contains('/') || line.contains(" UTC "),
        "missing IANA zone or UTC fallback: {line}"
    );
}

#[test]
fn datetime_section_appends_resolve_time_rule_only_when_tool_present() {
    // With `resolve_time` in the agent's tool set, the time-discipline rule
    // is rendered under the date block (prevents the LLM hand-computing epoch
    // timestamps — the bug this tool exists to fix).
    let with_tools: Vec<Box<dyn Tool>> =
        vec![Box::new(crate::openhuman::tools::ResolveTimeTool::new())];
    let with_prompt_tools = PromptTool::from_tools(&with_tools);
    let ctx_with = PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: &with_prompt_tools,
        workflows: &[],
        dispatcher_instructions: "instr",
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
    };
    let rendered_with = DateTimeSection.build(&ctx_with).unwrap();
    assert!(
        rendered_with.contains("resolve_time") && rendered_with.contains("never hand-compute"),
        "expected the resolve_time discipline rule when the tool is present; got:\n{rendered_with}"
    );

    // Without the tool, the rule must NOT appear (auto-scoping gate).
    let no_tools: Vec<Box<dyn Tool>> = vec![];
    let no_prompt_tools = PromptTool::from_tools(&no_tools);
    let ctx_without = PromptContext {
        tools: &no_prompt_tools,
        ..ctx_with
    };
    let rendered_without = DateTimeSection.build(&ctx_without).unwrap();
    assert!(
        !rendered_without.contains("never hand-compute"),
        "rule must be gated off when resolve_time is absent; got:\n{rendered_without}"
    );
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
    }
}

#[test]
fn user_identity_section_empty_when_unset() {
    let ctx = ctx_with_identity(None);
    let rendered = UserIdentitySection.build(&ctx).unwrap();
    assert!(rendered.is_empty());
}

#[test]
fn user_identity_section_renders_populated_fields_only() {
    let identity = UserIdentity {
        id: Some("u_42".to_string()),
        name: Some("Ada Lovelace".to_string()),
        email: None,
    };
    let ctx = ctx_with_identity(Some(identity));
    let rendered = UserIdentitySection.build(&ctx).unwrap();
    assert!(rendered.starts_with("## User\n\n"));
    assert!(rendered.contains("- name: Ada Lovelace"));
    assert!(rendered.contains("- id: u_42"));
    assert!(
        !rendered.contains("email:"),
        "empty email field must be skipped — leaking placeholders \
         confuses agents into asking the user to confirm them"
    );
}

#[test]
fn user_identity_section_skips_when_every_field_is_blank() {
    // Backend payloads that arrive with every field set to an empty
    // or whitespace string would otherwise pass the `is_empty()`
    // guard (None-only) and leave the prompt with an orphan
    // `## User` heading + intro paragraph pointing at zero fields —
    // exactly the failure mode the section is meant to suppress.
    let identity = UserIdentity {
        id: Some(String::new()),
        name: Some("   ".to_string()),
        email: Some("\t".to_string()),
    };
    let ctx = ctx_with_identity(Some(identity));
    let rendered = UserIdentitySection.build(&ctx).unwrap();
    assert!(
        rendered.is_empty(),
        "all-blank identity must produce no output, got:\n{rendered}"
    );
}

#[test]
fn user_identity_section_skips_blank_strings() {
    // Backend payloads sometimes carry empty-string fields rather than
    // null. Treat both the same so the prompt never renders
    // `- email: ` (which would invite the agent to "confirm" the
    // missing value with the user).
    let identity = UserIdentity {
        id: Some("   ".to_string()),
        name: Some(String::new()),
        email: Some("ada@example.com".to_string()),
    };
    let ctx = ctx_with_identity(Some(identity));
    let rendered = UserIdentitySection.build(&ctx).unwrap();
    assert!(rendered.starts_with("## User\n\n"));
    assert!(rendered.contains("- email: ada@example.com"));
    assert!(!rendered.contains("- name:"));
    assert!(!rendered.contains("- id:"));
}

#[test]
fn ambient_environment_orders_runtime_user_datetime() {
    let identity = UserIdentity {
        id: None,
        name: Some("Ada".to_string()),
        email: None,
    };
    let ctx = ctx_with_identity(Some(identity));
    let rendered = render_ambient_environment(&ctx).unwrap();
    let runtime_pos = rendered.find("## Runtime").expect("runtime missing");
    let user_pos = rendered.find("## User").expect("user missing");
    let dt_pos = rendered
        .find("## Current Date & Time")
        .expect("datetime missing");
    assert!(
        runtime_pos < user_pos && user_pos < dt_pos,
        "ambient block must order runtime → user → datetime so the \
         time-volatile section sits at the prompt tail (KV cache \
         convention from `with_defaults`); got:\n{rendered}"
    );
}

#[test]
fn tools_section_pformat_renders_signature_not_schema() {
    // ToolsSection must render `name[arg1|arg2]` signatures when
    // `tool_call_format = PFormat`, NOT the verbose JSON schema —
    // that's where most of the prompt token saving comes from.
    struct ParamTool;
    #[async_trait]
    impl Tool for ParamTool {
        fn name(&self) -> &str {
            "make_tea"
        }
        fn description(&self) -> &str {
            "brew a cup of tea"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string" },
                    "sugar": { "type": "boolean" }
                }
            })
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
        ) -> anyhow::Result<crate::openhuman::tools::ToolResult> {
            Ok(crate::openhuman::tools::ToolResult::success("ok"))
        }
    }

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(ParamTool)];
    let prompt_tools = PromptTool::from_tools(&tools);
    let ctx = PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: &prompt_tools,
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
    };

    let rendered = ToolsSection.build(&ctx).unwrap();
    // Alphabetical: kind, sugar.
    assert!(
        rendered.contains("Call as: `make_tea[kind|sugar]`"),
        "expected p-format signature in tools section, got:\n{rendered}"
    );
    // Should NOT contain the raw JSON schema dump.
    assert!(
        !rendered.contains("\"properties\""),
        "tools section should drop the raw JSON schema in p-format mode, got:\n{rendered}"
    );
}

#[test]
fn tools_section_uses_pformat_signature_for_text_dispatchers() {
    // Tool rendering is uniform across text dispatchers: always the
    // compact `Call as: name[args]` signature, never a raw JSON
    // schema dump. Native tool calls are handled differently — see
    // `tools_section_empty_for_native` below.
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let prompt_tools = PromptTool::from_tools(&tools);
    for format in [ToolCallFormat::PFormat, ToolCallFormat::Json] {
        let ctx = PromptContext {
            workspace_dir: Path::new("/tmp"),
            model_name: "test-model",
            agent_id: "",
            tools: &prompt_tools,
            workflows: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: &NO_FILTER,
            tool_call_format: format,
            connected_integrations: &[],
            connected_identities_md: String::new(),
            include_profile: false,
            include_memory_md: false,
            curated_snapshot: None,
            user_identity: None,
            personality_soul_md: None,
            personality_memory_md: None,
            personality_roster: vec![],
        };
        let rendered = ToolsSection.build(&ctx).unwrap();
        assert!(
            rendered.contains("Call as:"),
            "{format:?} must use the signature format, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("Parameters:"),
            "{format:?} should never emit the JSON `Parameters:` line, got:\n{rendered}"
        );
    }
}

#[test]
fn user_memory_section_renders_namespaces_with_headings() {
    let learned = LearnedContextData {
        tree_root_summaries: vec![
            ns_summary_at(
                "user",
                "Steven prefers terse Rust answers.",
                "2026-05-25T00:00:00Z",
            ),
            ns_summary_at(
                "conversations",
                "Recent thread: prompt rework.",
                "2026-05-25T00:00:00Z",
            ),
        ],
        ..Default::default()
    };
    let prompt_tools: Vec<PromptTool<'_>> = Vec::new();
    let ctx = PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: &prompt_tools,
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
    };
    let rendered = UserMemorySection.build(&ctx).unwrap();
    assert!(rendered.starts_with("## User Memory\n\n"));
    assert!(
        rendered
            .contains("### user (last updated 2026-05-25)\n\nSteven prefers terse Rust answers."),
        "heading must carry the absolute update date (#2944); got:\n{rendered}"
    );
    assert!(rendered
        .contains("### conversations (last updated 2026-05-25)\n\nRecent thread: prompt rework."));
}

#[test]
fn memory_date_label_formats_absolute_utc_date() {
    let dt = chrono::DateTime::parse_from_rfc3339("2026-05-25T18:30:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    // Absolute date, no time-of-day — must stay byte-stable day to day.
    assert_eq!(memory_date_label(dt), "2026-05-25");
}

#[test]
fn user_memory_section_labels_stale_summary_and_warns_against_present_tense() {
    // #2944 regression: a summary last updated weeks ago must render with
    // its absolute date, and the section must steer the model to compare
    // against the current date — so a May-25 briefing is never served as
    // today's.
    let learned = LearnedContextData {
        tree_root_summaries: vec![ns_summary_at(
            "briefings",
            "Daily briefing: 2 meetings, proposal due.",
            "2026-05-25T07:00:00Z",
        )],
        ..Default::default()
    };
    let rendered = UserMemorySection.build(&ctx_with_learned(learned)).unwrap();

    assert!(
        rendered.contains("### briefings (last updated 2026-05-25)"),
        "stale summary must carry its absolute update date; got:\n{rendered}"
    );
    // Guardrail: tell the model to cross-check against the current date
    // and not restate older memory as today's.
    assert!(
        rendered.contains("Current Date & Time"),
        "section must reference the current-date block; got:\n{rendered}"
    );
    assert!(
        rendered.contains("never present older memory as"),
        "section must forbid presenting stale memory as current; got:\n{rendered}"
    );
}

#[test]
fn user_memory_section_returns_empty_when_no_summaries() {
    // Empty learned context → section returns empty string and is
    // skipped by the prompt builder, so the cache boundary stays
    // exactly where it was for workspaces with no tree summaries.
    let learned = LearnedContextData::default();
    let prompt_tools: Vec<PromptTool<'_>> = Vec::new();
    let ctx = PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: &prompt_tools,
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
    };
    let rendered = UserMemorySection.build(&ctx).unwrap();
    assert!(rendered.is_empty());
}

#[test]
fn render_subagent_system_prompt_renders_workspace_tail() {
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_subagent_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are a focused sub-agent.",
        SubagentRenderOptions::narrow(),
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(rendered.contains("## Workspace"));
    assert!(rendered.contains("## Runtime"));
    // Grounding contract is appended even by the narrow (index-based)
    // sub-agent renderer — same source const, so it can never drift from
    // `GroundingSection` / the central `build()` append.
    assert!(rendered.contains("## Grounding and tool use"));
    assert!(rendered.contains("Your tools are exactly the ones listed in this prompt"));
    assert!(rendered.contains("Preserve numeric evidence exactly"));

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn subagent_render_options_invert_definition_flags() {
    // (omit_identity, omit_safety_preamble, omit_skills_catalog,
    //  omit_profile, omit_memory_md)
    let options = SubagentRenderOptions::from_definition_flags(true, false, true, false, false);
    assert!(!options.include_identity);
    assert!(options.include_safety_preamble);
    assert!(!options.include_skills_catalog);
    assert!(options.include_profile);
    assert!(options.include_memory_md);
    let narrow = SubagentRenderOptions::narrow();
    let default = SubagentRenderOptions::default();
    assert_eq!(narrow.include_identity, default.include_identity);
    assert_eq!(
        narrow.include_safety_preamble,
        default.include_safety_preamble
    );
    assert_eq!(
        narrow.include_skills_catalog,
        default.include_skills_catalog
    );
    assert_eq!(narrow.include_profile, default.include_profile);
    assert_eq!(narrow.include_memory_md, default.include_memory_md);
    // Narrow default = every flag off, including both user files.
    assert!(!narrow.include_profile);
    assert!(!narrow.include_memory_md);
}

#[test]
fn render_subagent_system_prompt_honors_identity_safety_and_skills_flags() {
    let workspace =
        std::env::temp_dir().join(format!("openhuman_prompt_opts_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("SOUL.md"), "# Soul\nContext").unwrap();
    std::fs::write(workspace.join("IDENTITY.md"), "# Identity\nContext").unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt_with_format(
        &workspace,
        "reasoning-v1",
        &[0],
        &tools,
        &[],
        "You are a specialist.",
        SubagentRenderOptions {
            include_identity: true,
            include_safety_preamble: true,
            include_skills_catalog: true,
            include_profile: false,
            include_memory_md: false,
        },
        ToolCallFormat::Json,
        &[],
    );

    assert!(rendered.contains("## Project Context"));
    assert!(rendered.contains("### SOUL.md"));
    assert!(rendered.contains("## Safety"));
    // Json is a prompt-driven format (the model wraps JSON tool
    // calls in `<tool_call>` tags); it does NOT use the provider's
    // native function-calling channel. So the prose `## Tools`
    // section MUST still be rendered for Json, with each tool's
    // parameter schema inline so the model knows what to emit.
    // Only `ToolCallFormat::Native` gets the section omitted (see
    // the `native` branch below and the `!matches!(…, Native)`
    // guard in the renderer).
    assert!(rendered.contains("## Tools"));
    assert!(rendered.contains("Parameters:"));
    assert!(rendered.contains("\"type\""));

    let native = render_subagent_system_prompt_with_format(
        &workspace,
        "reasoning-v1",
        &[0],
        &tools,
        &[],
        "You are a specialist.",
        SubagentRenderOptions::narrow(),
        ToolCallFormat::Native,
        &[],
    );
    assert!(native.contains("native tool-calling output"));
    assert!(!native.contains("## Safety"));
    // Native is the only format where the prose `## Tools` section
    // is intentionally omitted — schemas travel through the
    // provider's `tools` field instead. Regression guard against
    // the ~54k-token schema duplication from the #447 PR.
    assert!(!native.contains("\n## Tools\n"));
    assert!(!native.contains("Parameters:"));

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn render_subagent_system_prompt_injects_profile_md_even_when_identity_omitted() {
    // Regression: an agent with `omit_identity = true` drops the SOUL/IDENTITY
    // preamble but still needs PROFILE.md if `include_profile = true`.
    // PROFILE.md is gated on its own flag so agents can opt in without
    // pulling SOUL/IDENTITY back in.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_profile_nosoul_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("SOUL.md"), "# Soul\nShould be hidden").unwrap();
    std::fs::write(
        workspace.join("IDENTITY.md"),
        "# Identity\nShould be hidden",
    )
    .unwrap();
    std::fs::write(
        workspace.join("PROFILE.md"),
        "# User Profile\nName: Jane Doe\nRole: Data scientist",
    )
    .unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are a specialist agent.",
        SubagentRenderOptions {
            include_identity: false,
            include_safety_preamble: false,
            include_skills_catalog: false,
            include_profile: true,
            include_memory_md: false,
        },
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(
        rendered.contains("### PROFILE.md"),
        "PROFILE.md header must appear when include_profile=true, got:\n{rendered}"
    );
    assert!(
        rendered.contains("Jane Doe"),
        "PROFILE.md body must be injected when include_profile=true, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("## Project Context"),
        "identity preamble must still be suppressed when include_identity=false"
    );
    assert!(
        !rendered.contains("### SOUL.md") && !rendered.contains("### IDENTITY.md"),
        "SOUL/IDENTITY must still be suppressed when include_identity=false"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn render_subagent_system_prompt_skips_profile_md_when_include_profile_false() {
    // Mirror of the opt-in regression above: narrow specialists
    // (planner, code_executor, critic, …) set `omit_profile = true`
    // and must NOT see PROFILE.md even when the file is on disk —
    // otherwise every sub-agent pays the token cost of onboarding
    // enrichment output that is irrelevant to their task.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_profile_opt_out_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("PROFILE.md"),
        "# User Profile\nName: Jane Doe\nRole: Data scientist",
    )
    .unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are a narrow specialist.",
        SubagentRenderOptions::narrow(), // include_profile defaults to false
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(
        !rendered.contains("### PROFILE.md"),
        "PROFILE.md must NOT appear when include_profile=false, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("Jane Doe"),
        "PROFILE.md body must NOT be leaked when include_profile=false"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn render_subagent_system_prompt_injects_profile_md_when_identity_included() {
    // When identity is on, PROFILE.md must still be injected alongside
    // SOUL/IDENTITY — the split must not regress the non-welcome path.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_profile_with_identity_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("SOUL.md"), "# Soul\nctx").unwrap();
    std::fs::write(workspace.join("IDENTITY.md"), "# Identity\nctx").unwrap();
    std::fs::write(workspace.join("PROFILE.md"), "# User Profile\nhello").unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are a specialist.",
        SubagentRenderOptions {
            include_identity: true,
            include_safety_preamble: false,
            include_skills_catalog: false,
            include_profile: true,
            include_memory_md: false,
        },
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(rendered.contains("## Project Context"));
    assert!(rendered.contains("### SOUL.md"));
    assert!(rendered.contains("### IDENTITY.md"));
    assert!(rendered.contains("### PROFILE.md"));
    assert!(rendered.contains("hello"));

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn render_subagent_system_prompt_silently_skips_missing_profile_md() {
    // Pre-onboarding workspaces have no PROFILE.md. The renderer must
    // not emit a noisy "[File not found: PROFILE.md]" placeholder or
    // an orphan "### PROFILE.md" header — the subagent prompt stays
    // focused on tools.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_profile_missing_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are a specialist agent.",
        SubagentRenderOptions::narrow(),
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(
        !rendered.contains("### PROFILE.md"),
        "empty/missing PROFILE.md should not emit a header, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("[File not found: PROFILE.md]"),
        "missing PROFILE.md should be silent, not a noisy placeholder"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn narrow_agent_with_omit_identity_still_loads_profile_md() {
    // Verify that an agent configured with omit_identity=true/omit_skills_catalog=true/
    // omit_safety_preamble=true/omit_profile=false still gets PROFILE.md injected.
    // This exercises the SubagentRenderOptions::from_definition_flags path for agents
    // that want PROFILE.md without the full SOUL/IDENTITY preamble.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_narrow_agent_flags_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("PROFILE.md"),
        "# User Profile\nTimezone: PST\nRole: Crypto trader",
    )
    .unwrap();

    let options = SubagentRenderOptions::from_definition_flags(
        true,  // omit_identity
        true,  // omit_safety_preamble
        true,  // omit_skills_catalog
        false, // omit_profile   — opts IN to PROFILE.md
        false, // omit_memory_md — opts IN to MEMORY.md too
    );

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "# Specialist Agent\n\nYou are a specialist.",
        options,
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(
        rendered.contains("### PROFILE.md"),
        "agent with omit_profile=false must load PROFILE.md, got:\n{rendered}"
    );
    assert!(
        rendered.contains("Crypto trader"),
        "PROFILE.md body must reach the agent prompt"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn narrow_subagent_definition_flags_skip_profile_md() {
    // Inverse of `welcome_agent_definition_flags_still_load_profile_md`:
    // a narrow specialist (e.g. `code_executor`, `critic`) leaves
    // `omit_profile` at its default `true`. PROFILE.md must NOT be
    // injected even when present on disk — the narrow runner is
    // task-focused and should not pay the token cost.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_narrow_flags_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("PROFILE.md"),
        "# User Profile\nTimezone: PST\nRole: Crypto trader",
    )
    .unwrap();

    // Mirrors e.g. `critic/agent.toml` — all omit_* default-true.
    let options = SubagentRenderOptions::from_definition_flags(true, true, true, true, true);

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are a narrow specialist.",
        options,
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(
        !rendered.contains("### PROFILE.md"),
        "narrow specialist (omit_profile=true) must NOT load PROFILE.md, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("Crypto trader"),
        "narrow specialist must not leak PROFILE.md body"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn render_subagent_system_prompt_injects_memory_md_when_enabled() {
    // Opt-in agents with `omit_memory_md = false` must see MEMORY.md
    // (archivist-curated long-term memory) in their rendered prompt.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_memory_on_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("MEMORY.md"),
        "# Long-term memory\nUser prefers terse Rust answers.",
    )
    .unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are a specialist agent.",
        SubagentRenderOptions {
            include_identity: false,
            include_safety_preamble: false,
            include_skills_catalog: false,
            include_profile: false,
            include_memory_md: true,
        },
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(
        rendered.contains("### MEMORY.md"),
        "MEMORY.md header must appear when include_memory_md=true, got:\n{rendered}"
    );
    assert!(
        rendered.contains("terse Rust answers"),
        "MEMORY.md body must be injected when include_memory_md=true"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn render_subagent_system_prompt_skips_memory_md_when_disabled() {
    // Narrow specialists with `omit_memory_md = true` (the default)
    // must NOT see MEMORY.md even when it exists on disk.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_memory_off_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("MEMORY.md"),
        "# Long-term memory\nUser prefers terse Rust answers.",
    )
    .unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are a narrow specialist.",
        SubagentRenderOptions::narrow(),
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(
        !rendered.contains("### MEMORY.md"),
        "MEMORY.md must NOT appear when include_memory_md=false, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("terse Rust answers"),
        "MEMORY.md body must not leak when include_memory_md=false"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn profile_md_and_memory_md_are_capped_at_user_file_max_chars() {
    // Both PROFILE.md and MEMORY.md are user-specific files that can
    // grow over time. Injection caps them at USER_FILE_MAX_CHARS
    // (~1000 tokens each) so the system prompt footprint stays
    // bounded. Test both files at once to pin the shared budget.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_user_cap_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let big = "x".repeat(USER_FILE_MAX_CHARS + 500);
    std::fs::write(workspace.join("PROFILE.md"), &big).unwrap();
    std::fs::write(workspace.join("MEMORY.md"), &big).unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let rendered = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are the orchestrator.",
        SubagentRenderOptions {
            include_identity: false,
            include_safety_preamble: false,
            include_skills_catalog: false,
            include_profile: true,
            include_memory_md: true,
        },
        ToolCallFormat::PFormat,
        &[],
    );

    assert!(rendered.contains("### PROFILE.md"));
    assert!(rendered.contains("### MEMORY.md"));
    // Each file gets its own truncation marker mentioning the cap.
    let marker = format!("[... truncated at {USER_FILE_MAX_CHARS} chars");
    assert_eq!(
        rendered.matches(marker.as_str()).count(),
        2,
        "both PROFILE.md and MEMORY.md must emit the truncation marker at \
         USER_FILE_MAX_CHARS — found:\n{rendered}"
    );
    // Sanity-check the cap is genuinely tighter than the bootstrap cap.
    assert!(USER_FILE_MAX_CHARS < BOOTSTRAP_MAX_CHARS);

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn rendered_subagent_system_prompt_is_byte_stable_across_repeat_calls() {
    // KV-cache contract: two spawns of the same sub-agent definition
    // against the same workspace must produce byte-identical system
    // prompts. If PROFILE.md or MEMORY.md are re-read with a
    // different-typed truncation path, or if either cap drifts, the
    // bytes differ and the backend's automatic prefix cache busts.
    // This test pins the invariant end-to-end.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_byte_stable_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("PROFILE.md"), "# User Profile\nJane Doe").unwrap();
    std::fs::write(workspace.join("MEMORY.md"), "# Memory\nRecent: shipped v1").unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let opts = SubagentRenderOptions {
        include_identity: false,
        include_safety_preamble: false,
        include_skills_catalog: false,
        include_profile: true,
        include_memory_md: true,
    };

    let first = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are the orchestrator.",
        opts,
        ToolCallFormat::PFormat,
        &[],
    );
    let second = render_subagent_system_prompt(
        &workspace,
        "test-model",
        &[0],
        &tools,
        &[],
        "You are the orchestrator.",
        opts,
        ToolCallFormat::PFormat,
        &[],
    );

    assert_eq!(
        first, second,
        "repeat spawns must produce byte-identical prompts"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn for_subagent_builder_injects_user_files_even_when_identity_omitted() {
    // Regression pin for the review finding: the runtime Tauri chat
    // path spins welcome/trigger_* via `Agent::from_config_for_agent`
    // → `SystemPromptBuilder::for_subagent(body, omit_identity=true, …)`,
    // which deliberately drops `IdentitySection`. Before
    // `UserFilesSection` existed, our PROFILE/MEMORY injection lived
    // inside `IdentitySection::build` and got dropped along with it,
    // so the first Tauri turn never saw the user's onboarding output
    // even though the subagent_runner path and the debug dumper did.
    //
    // This test exercises the exact builder call-site the runtime
    // uses for welcome (`omit_identity = true`, both user-file flags
    // opted in via PromptContext) and pins that the rendered prompt
    // contains both files.
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_for_subagent_user_files_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("PROFILE.md"),
        "# User Profile\nJane Doe — crypto trader in PST.",
    )
    .unwrap();
    std::fs::write(
        workspace.join("MEMORY.md"),
        "# Long-term memory\nShipped v1 last sprint; prefers terse Rust.",
    )
    .unwrap();

    let tools: Vec<Box<dyn Tool>> = vec![];
    let prompt_tools = PromptTool::from_tools(&tools);
    let ctx = PromptContext {
        workspace_dir: &workspace,
        model_name: "test-model",
        agent_id: "",
        tools: &prompt_tools,
        workflows: &[],
        dispatcher_instructions: "",
        learned: LearnedContextData::default(),
        visible_tool_names: &NO_FILTER,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        include_profile: true,
        include_memory_md: true,
        curated_snapshot: None,
        user_identity: None,
        personality_soul_md: None,
        personality_memory_md: None,
        personality_roster: vec![],
    };

    // Test a narrow-agent runtime path:
    // `SystemPromptBuilder::for_subagent(body, omit_identity=true, …)`.
    let builder = SystemPromptBuilder::for_subagent(
        "You are a specialist agent.".into(),
        true, // omit_identity  — drops SOUL/IDENTITY preamble
        true, // omit_safety_preamble
        true, // omit_skills_catalog
    );
    let rendered = builder.build(&ctx).unwrap();

    assert!(
        !rendered.contains("## Project Context"),
        "identity preamble must still be suppressed when omit_identity=true"
    );
    assert!(
        rendered.contains("### PROFILE.md") && rendered.contains("Jane Doe"),
        "narrow runtime path must inject PROFILE.md despite omit_identity=true, got:\n{rendered}"
    );
    assert!(
        rendered.contains("### MEMORY.md") && rendered.contains("terse Rust"),
        "narrow runtime path must inject MEMORY.md despite omit_identity=true, got:\n{rendered}"
    );

    // Mirror the narrow-specialist runtime path (code_executor,
    // critic, …): both flags off → user files must stay out.
    let ctx_narrow = PromptContext {
        workspace_dir: &workspace,
        model_name: "test-model",
        agent_id: "",
        tools: &prompt_tools,
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
    };
    let narrow = builder.build(&ctx_narrow).unwrap();
    assert!(
        !narrow.contains("### PROFILE.md") && !narrow.contains("### MEMORY.md"),
        "narrow specialist runtime path must NOT leak user files, got:\n{narrow}"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn sync_workspace_file_updates_hash_and_inject_workspace_file_truncates() {
    let workspace = std::env::temp_dir().join(format!(
        "openhuman_prompt_workspace_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();

    sync_workspace_file(&workspace, "SOUL.md");
    let hash_path = workspace.join(".SOUL.md.builtin-hash");
    assert!(workspace.join("SOUL.md").exists());
    assert!(hash_path.exists());
    let original_hash = std::fs::read_to_string(&hash_path).unwrap();

    std::fs::write(workspace.join("SOUL.md"), "user override").unwrap();
    sync_workspace_file(&workspace, "SOUL.md");
    assert_eq!(std::fs::read_to_string(&hash_path).unwrap(), original_hash);
    assert_eq!(
        std::fs::read_to_string(workspace.join("SOUL.md")).unwrap(),
        "user override"
    );

    std::fs::write(
        workspace.join("BIG.md"),
        "x".repeat(BOOTSTRAP_MAX_CHARS + 50),
    )
    .unwrap();
    let mut prompt = String::new();
    inject_workspace_file(&mut prompt, &workspace, "BIG.md");
    assert!(prompt.contains("### BIG.md"));
    assert!(prompt.contains("[... truncated at"));

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn prompt_tool_constructors_and_user_memory_skip_empty_bodies() {
    let plain = PromptTool::new("shell", "run commands");
    assert_eq!(plain.name, "shell");
    assert!(plain.parameters_schema.is_none());

    let with_schema =
        PromptTool::with_schema("http_request", "fetch data", "{\"type\":\"object\"}".into());
    assert_eq!(
        with_schema.parameters_schema.as_deref(),
        Some("{\"type\":\"object\"}")
    );

    let ctx = PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "model",
        agent_id: "",
        tools: &[],
        workflows: &[],
        dispatcher_instructions: "",
        learned: LearnedContextData {
            tree_root_summaries: vec![ns_summary("user", "kept"), ns_summary("empty", "   ")],
            ..Default::default()
        },
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
    };
    let rendered = UserMemorySection.build(&ctx).unwrap();
    assert!(rendered.contains("### user"));
    assert!(!rendered.contains("### empty"));
    assert_eq!(default_workspace_file_content("missing"), "");
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
    }
}

#[test]
fn user_reflections_section_renders_bullets_with_priority_preamble() {
    let ctx = ctx_with_learned(LearnedContextData {
        reflections: vec![
            "Going forward I want concise replies".into(),
            "I realized I prefer Rust over TypeScript".into(),
        ],
        ..Default::default()
    });
    let rendered = UserReflectionsSection.build(&ctx).unwrap();
    assert!(rendered.starts_with("## User Reflections\n\n"));
    assert!(
        rendered.contains("higher-priority"),
        "preamble must signal that reflections outrank generic memory"
    );
    assert!(rendered.contains("- Going forward I want concise replies"));
    assert!(rendered.contains("- I realized I prefer Rust over TypeScript"));
}

#[test]
fn user_reflections_section_returns_empty_without_entries() {
    let ctx = ctx_with_learned(LearnedContextData::default());
    assert!(UserReflectionsSection.build(&ctx).unwrap().is_empty());
}

#[test]
fn user_reflections_section_skips_blank_entries() {
    let ctx = ctx_with_learned(LearnedContextData {
        reflections: vec!["   ".into(), "Real reflection".into(), "".into()],
        ..Default::default()
    });
    let rendered = UserReflectionsSection.build(&ctx).unwrap();
    assert!(rendered.contains("- Real reflection"));
    // Bullet count should match the non-blank entry count.
    assert_eq!(rendered.matches("\n- ").count(), 1);
}

#[test]
fn render_user_reflections_helper_matches_section_output() {
    let ctx = ctx_with_learned(LearnedContextData {
        reflections: vec!["x".into()],
        ..Default::default()
    });
    let via_section = UserReflectionsSection.build(&ctx).unwrap();
    let via_helper = render_user_reflections(&ctx).unwrap();
    assert_eq!(via_section, via_helper);
}

#[test]
fn insert_section_before_places_section_ahead_of_named_target() {
    // Reflections must rank ahead of generic memory in builders that
    // already include `UserMemorySection` (the `with_defaults` chain).
    // Verify the helper inserts at the correct index instead of
    // tail-appending.
    let builder = SystemPromptBuilder::with_defaults()
        .insert_section_before("user_memory", Box::new(UserReflectionsSection));
    let names: Vec<&str> = builder.sections.iter().map(|s| s.name()).collect();
    let r_idx = names
        .iter()
        .position(|n| *n == "user_reflections")
        .expect("user_reflections section");
    let m_idx = names
        .iter()
        .position(|n| *n == "user_memory")
        .expect("user_memory section");
    assert!(
        r_idx < m_idx,
        "insert_section_before should place the new section ahead of its target, got order {names:?}"
    );
}

#[test]
fn insert_section_before_falls_back_to_append_when_target_missing() {
    // Dynamic / sub-agent builders do not include a `user_memory`
    // section. The helper should still land the new section so the
    // caller's wiring stays loop-free, just at the tail.
    let builder = SystemPromptBuilder::default()
        .add_section(Box::new(SafetySection))
        .insert_section_before("user_memory", Box::new(UserReflectionsSection));
    let names: Vec<&str> = builder.sections.iter().map(|s| s.name()).collect();
    assert_eq!(names.last(), Some(&"user_reflections"));
    assert_eq!(names.len(), 2);
}

#[test]
fn user_reflections_render_above_user_memory_when_both_present() {
    // Acceptance criterion: reflections rank above generic
    // tree summaries — verify by composing the same way the runtime
    // does (UserReflectionsSection appended ahead of any
    // UserMemorySection content).
    let ctx = ctx_with_learned(LearnedContextData {
        reflections: vec!["I want terse answers".into()],
        tree_root_summaries: vec![ns_summary("user", "Generic summary")],
        ..Default::default()
    });
    let reflections = UserReflectionsSection.build(&ctx).unwrap();
    let memory = UserMemorySection.build(&ctx).unwrap();
    let combined = format!("{reflections}{memory}");
    let r_idx = combined
        .find("## User Reflections")
        .expect("reflections heading");
    let m_idx = combined.find("## User Memory").expect("memory heading");
    assert!(
        r_idx < m_idx,
        "reflections must render before user-memory block"
    );
}

// ─── ToolsSection native-skip tests ──────────────────────────────────────────

#[test]
fn tools_section_empty_for_native() {
    // Native function-calling: the provider sends full JSON schemas in the
    // API request — repeating them in the system prompt is pure token bloat.
    // ToolsSection must return an empty string for Native mode.
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let prompt_tools = PromptTool::from_tools(&tools);
    let ctx = PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: &prompt_tools,
        workflows: &[],
        dispatcher_instructions: "",
        learned: LearnedContextData::default(),
        visible_tool_names: &NO_FILTER,
        tool_call_format: ToolCallFormat::Native,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
        user_identity: None,
        personality_soul_md: None,
        personality_memory_md: None,
        personality_roster: vec![],
    };
    let out = ToolsSection.build(&ctx).unwrap();
    assert!(
        out.is_empty(),
        "Native mode should produce empty ToolsSection, got: {out:?}"
    );
}

#[test]
fn tools_section_nonempty_for_pformat() {
    // PFormat is a text-driven format — the model discovers tools by reading
    // the prose `## Tools` section. It must be non-empty.
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let prompt_tools = PromptTool::from_tools(&tools);
    let ctx = PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: &prompt_tools,
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
    };
    let out = ToolsSection.build(&ctx).unwrap();
    assert!(
        out.contains("## Tools"),
        "PFormat should render tool catalogue header, got: {out:?}"
    );
}

#[test]
fn tools_section_native_with_dispatcher_instructions_returns_instructions() {
    // Native mode must still include non-empty dispatcher_instructions
    // (e.g. the "## Tool Use Protocol" block from NativeToolDispatcher) so
    // the model receives behavioural guidance even though the tool catalogue
    // itself is omitted.
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
    let prompt_tools = PromptTool::from_tools(&tools);
    let ctx = PromptContext {
        workspace_dir: Path::new("/tmp"),
        model_name: "test-model",
        agent_id: "",
        tools: &prompt_tools,
        workflows: &[],
        dispatcher_instructions: "## Tool Use Protocol\n\nUse native tool calling.",
        learned: LearnedContextData::default(),
        visible_tool_names: &NO_FILTER,
        tool_call_format: ToolCallFormat::Native,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
        user_identity: None,
        personality_soul_md: None,
        personality_memory_md: None,
        personality_roster: vec![],
    };
    let out = ToolsSection.build(&ctx).unwrap();
    assert!(
        out.contains("## Tool Use Protocol"),
        "Native mode with non-empty dispatcher_instructions must include them, got: {out:?}"
    );
    assert!(
        !out.contains("## Tools"),
        "Native mode must not include the tool catalogue header, got: {out:?}"
    );
}
