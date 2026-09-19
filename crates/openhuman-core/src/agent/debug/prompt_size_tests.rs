use super::*;

#[test]
fn sections_sum_to_the_whole_prompt() {
    let text = "preamble line\n# Title\nbody\n## Sub\nmore body\n### Deep\ntail\n";
    let sections = split_sections(text);
    let total: usize = sections.iter().map(|s| s.bytes).sum();
    assert_eq!(
        total,
        text.len(),
        "section bytes must account for every byte of the prompt; \
             a table that does not sum is worse than no table"
    );
}

#[test]
fn preamble_is_kept_when_text_starts_before_the_first_heading() {
    let sections = split_sections("loose text\n# Title\nbody\n");
    assert_eq!(sections[0].heading, "(preamble)");
    assert_eq!(sections[0].bytes, "loose text\n".len());
}

#[test]
fn no_preamble_entry_when_the_prompt_opens_on_a_heading() {
    let sections = split_sections("# Title\nbody\n");
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].heading, "# Title");
}

#[test]
fn deeper_headings_do_not_split() {
    // `####` is body text as far as this report is concerned.
    let sections = split_sections("## Sub\na\n#### Deeper\nb\n");
    assert_eq!(sections.len(), 1);
}

#[test]
fn a_bare_hash_is_not_a_heading() {
    // No trailing space: a shell comment in an example block, not a section.
    assert!(!is_heading("#!/usr/bin/env bash\n"));
    assert!(!is_heading("#hashtag\n"));
    assert!(is_heading("## Real\n"));
}

#[test]
fn tools_are_ranked_by_cost_not_registration_order() {
    let dumped = DumpedPrompt {
        agent_id: "t".into(),
        toolkit: None,
        mode: "session",
        model: "m".into(),
        workspace_dir: std::path::PathBuf::from("/tmp"),
        text: "# A\nbody\n".into(),
        tool_names: vec!["small".into(), "big".into()],
        skill_tool_count: 0,
        tool_specs: vec![
            serde_json::json!({"name": "small", "description": "s", "parameters": {}}),
            serde_json::json!({
                "name": "big",
                "description": "a much longer description than the other one",
                "parameters": {"type": "object", "properties": {"a": {"type": "string"}}}
            }),
        ],
    };
    let report = PromptSizeReport::from_dump(&dumped);
    assert_eq!(report.tools[0].name, "big");
    assert_eq!(report.tool_count, 2);
    assert_eq!(
        report.fixed_prefix_bytes,
        report.prompt_bytes + report.tool_bytes
    );
    assert!(report.tools[1].parameters_bytes > 0, "`{{}}` is two bytes");
}

/// The report must measure the tool schemas the provider receives — the
/// agent's visible set — not the whole registry. d149ab0f0 switched the
/// session dump to `all_tool_refs()`, and every agent then reported the same
/// ~200 tools, so every `tool_bytes` budget described a belt no agent carries.
#[test]
fn session_report_measures_the_visible_belt_not_the_registry() {
    crate::agent::harness::AgentDefinitionRegistry::init_global_builtins().unwrap();
    let tmp = tempfile::TempDir::new().unwrap();
    let config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..crate::config::Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    let agent = crate::agent::OpenHumanSessionHost::from_config_for_agent(&config, "critic")
        .expect("critic session build");

    let dumped = crate::agent::debug::session_dump(&agent, "critic", String::new());
    let report = PromptSizeReport::from_dump(&dumped);

    let mut visible: Vec<String> = agent
        .visible_tool_specs_arc()
        .iter()
        .map(|s| s.name.clone())
        .collect();
    visible.sort();
    let mut reported: Vec<String> = report.tools.iter().map(|t| t.name.clone()).collect();
    reported.sort();
    assert_eq!(
        reported, visible,
        "prompt-size must report the provider-facing visible tool set"
    );
    assert!(
        report.tool_count < dumped.tool_names.len(),
        "critic's belt ({}) must be narrower than its registry ({}), or this test \
         cannot tell the two apart",
        report.tool_count,
        dumped.tool_names.len()
    );
}
