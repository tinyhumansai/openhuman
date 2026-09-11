use super::*;

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
        agents_md_global: None,
        agents_md_local: None,
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
        agents_md_global: None,
        agents_md_local: None,
    };

    // A distinctive clause from GROUNDING_BODY — present regardless of which
    // builder produced the prompt (single source of truth, central append).
    let marker = "Your tools are exactly the ones listed in this prompt";

    // 1. Static default chain.
    let defaults = SystemPromptBuilder::with_defaults().build(&ctx).unwrap();
    assert!(defaults.contains("## Grounding and tool use"));
    assert!(defaults.contains(marker));

    // 2. Sub-agent static chain.
    let sub = SystemPromptBuilder::for_subagent("role".into(), true, true)
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
    let s = defaults.find("# Writing style").unwrap();
    assert!(g < s, "grounding should precede the writing-style suffix");
}

#[test]
fn grounding_contract_requires_exact_numeric_evidence() {
    let ctx = ctx_with_identity(None);
    let rendered = SystemPromptBuilder::from_final_body("## Custom Agent\n\nBody.".into())
        .build(&ctx)
        .unwrap();

    // WORDING LOCK (deliberate, plan.md §3): pin ONE representative clause of
    // the numeric-evidence grounding rule so a copy edit that silently drops
    // the "preserve numbers exactly" guidance trips review — rather than five
    // verbatim prose substrings that break on any harmless rewording. The
    // *structural* guarantee (the grounding contract is appended on every build
    // path) is covered behaviourally by
    // grounding_contract_appended_to_every_build_path. Update this string only
    // on a deliberate rewrite of GROUNDING_BODY.
    assert!(
        rendered.contains("Preserve numeric evidence exactly"),
        "numeric-evidence grounding clause missing from the built prompt"
    );
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
        agents_md_global: None,
        agents_md_local: None,
    };

    let section = IdentitySection;
    let _ = section.build(&ctx).unwrap();

    for file in ["SOUL.md", "IDENTITY.md", "ROLE.md"] {
        assert!(
            workspace.join(file).exists(),
            "expected workspace file to be created: {file}"
        );
    }
    // HEARTBEAT.md and MEMORY_GOALS.md are no longer seeded (#5701). The
    // subconscious engine that read HEARTBEAT.md is gone, and the goals store
    // returns an empty `GoalsDoc` for a missing file and creates it on first
    // write, so seeding either bought a file nothing needed.
    for file in ["HEARTBEAT.md", "MEMORY_GOALS.md"] {
        assert!(
            !workspace.join(file).exists(),
            "retired workspace file must not be seeded: {file}"
        );
    }
    // Seeded SOUL.md must equal the checked-in template verbatim (plan.md §3):
    // compare against the embedded template rather than pinning brand-voice
    // prose here — a missing file is seeded straight from
    // default_workspace_file_content, which is this same `include_str!`.
    let soul = std::fs::read_to_string(workspace.join("SOUL.md")).unwrap();
    assert_eq!(
        soul,
        include_str!("SOUL.md"),
        "seeded SOUL.md must be the checked-in template verbatim"
    );

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn soul_template_carries_brand_voice_guardrail() {
    // BRAND-VOICE LOCK (#3604, plan.md §3): a narrow, deliberately-labeled
    // wording pin on the *source* SOUL.md template — the constructive-defense
    // guardrail must survive edits so the agent defends the product instead of
    // validating FUD. Update only on an intentional brand-voice change.
    let soul = include_str!("SOUL.md");
    assert!(
        soul.contains("## When OpenHuman is criticized"),
        "SOUL.md must carry the brand-voice section (#3604)"
    );
    assert!(
        soul.contains("Don't validate FUD"),
        "SOUL.md brand-voice section must keep the do-not-validate-FUD directive (#3604)"
    );
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
        agents_md_global: None,
        agents_md_local: None,
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
    let line = super::super::current_datetime_line();
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
        agents_md_global: None,
        agents_md_local: None,
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
        agents_md_global: None,
        agents_md_local: None,
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
            agents_md_global: None,
            agents_md_local: None,
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
        agents_md_global: None,
        agents_md_local: None,
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
