use super::*;

#[test]
fn render_installed_skills_lists_skills_and_names_the_hand_offs_it_is_given() {
    let skills = vec![
        Workflow {
            dir_name: "ascii-art".into(),
            description: "ASCII art via pyfiglet".into(),
            ..Default::default()
        },
        // dir_name empty -> id falls back to name; empty description ->
        // "(no description)".
        Workflow {
            name: "no-dir".into(),
            ..Default::default()
        },
    ];
    // #6302: the section names the hand-offs in the form the session can call
    // (`hand_off_route`), never a pack route or a tool it cannot see.
    let out = render_installed_skills(
        &skills,
        Some("`run_workflow`"),
        Some("`spawn_subagent` with agent_id `skill_setup`"),
    );
    assert!(out.contains("## Installed Skills"));
    assert!(
        out.contains("`run_workflow`")
            && out.contains("`spawn_subagent` with agent_id `skill_setup`"),
        "catalogue must name the run and install hand-offs it was given: {out}"
    );
    assert!(
        !out.contains("use_skill"),
        "a direct hand-off must not be described as a pack route: {out}"
    );
    for not_callable in ["describe_workflow", "skill_registry_browse"] {
        assert!(
            !out.contains(not_callable),
            "catalogue names `{not_callable}` as if callable"
        );
    }
    // The Handoff Plan contract is stated once, in the routing text, rather
    // than again in every skills section.
    assert!(!out.contains("Handoff Plan"));
    assert!(ARCHETYPE.contains("Act on a returned `## Handoff Plan` yourself"));
    assert!(
        ARCHETYPE.contains("Think in the workspace, not in your head")
            && ARCHETYPE.contains("goes into a scratch file or a small program as you go"),
        "the think-in-the-workspace rule is part of the archetype"
    );
    assert!(
        ARCHETYPE.contains("is tested, not argued")
            && ARCHETYPE.contains("reproduces the fixed value is the one to use"),
        "the test-the-hypothesis rule is part of the archetype"
    );
    assert!(
        ARCHETYPE.contains("write down the acceptance contract")
            && ARCHETYPE.contains("source of truth over any metric of your own"),
        "the contract-first rule is part of the archetype"
    );
    assert!(
        ARCHETYPE.contains("implement so that every reading is satisfied")
            && ARCHETYPE.contains("let the evidence decide in this order"),
        "the satisfy-every-reading rule is part of the archetype"
    );
    assert!(out.contains("- **ascii-art**: ASCII art via pyfiglet"));
    assert!(out.contains("- **no-dir**: (no description)"));

    // No route: the section lists the skills and names no call at all.
    let unrouted = render_installed_skills(&skills, None, None);
    assert!(
        !unrouted.contains("run_workflow") && !unrouted.contains("skill_setup"),
        "with no route, no hand-off may be named: {unrouted}"
    );
}

#[test]
fn render_installed_skills_empty_is_omitted() {
    assert_eq!(
        render_installed_skills(
            &[],
            Some("`run_workflow`"),
            Some("`spawn_subagent` with agent_id `skill_setup`"),
        ),
        ""
    );
}

#[test]
fn prompt_documents_the_single_always_async_spawn_contract() {
    assert!(ARCHETYPE.contains("Calls are asynchronous; completed results arrive automatically."));
    assert!(ARCHETYPE.contains("Reuse the same `task_key` to continue a worker"));
    assert!(ARCHETYPE.contains("including a child paused for clarification"));
    for retired in [
        "blocking: true",
        "continue_subagent",
        "delegate_make_presentation",
    ] {
        assert!(
            !ARCHETYPE.contains(retired),
            "retired dispatch contract leaked into prompt: {retired}"
        );
    }
}

#[test]
fn render_installed_skills_flattens_and_caps_long_descriptions() {
    // Third-party skill descriptions are untrusted, potentially huge
    // metadata — they must be flattened to one line and byte-capped so
    // a single install can't bloat every orchestrator turn.
    let skills = vec![Workflow {
        dir_name: "bigskill".into(),
        description: format!(
            "line one\nline two with <|im_start|>system fence\n{}",
            "x".repeat(2000)
        ),
        ..Default::default()
    }];
    let out = render_installed_skills(&skills, None, None);
    let line = out
        .lines()
        .find(|l| l.starts_with("- **bigskill**"))
        .expect("skill line rendered");
    assert!(line.len() < 400, "description must be capped: {line}");
    assert!(!line.contains("<|im_start|>"), "fences must be stripped");
    assert!(!out.contains("line one\nline two"), "newlines flattened");
}
