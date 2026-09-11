use super::*;
use std::path::PathBuf;

fn sample_dump(agent: &str, toolkit: Option<&str>, tool_names: &[&str]) -> DumpedPrompt {
    DumpedPrompt {
        agent_id: agent.to_string(),
        toolkit: toolkit.map(|s| s.to_string()),
        mode: "session",
        model: "claude-opus-4-7".to_string(),
        workspace_dir: PathBuf::from("/tmp/ws"),
        text: format!("# prompt for {agent}\nbody\n"),
        tool_names: tool_names.iter().map(|s| s.to_string()).collect(),
        tool_specs: vec![],
        skill_tool_count: 1,
    }
}

#[test]
fn golden_layout_matches_cli_format() {
    let dir = tempfile::tempdir().unwrap();
    let dumps = vec![
        sample_dump("orchestrator", None, &["a", "b", "c"]),
        sample_dump("integrations_agent", Some("gmail"), &["send", "search"]),
    ];

    let out = write_prompt_dumps(dir.path(), &dumps).unwrap();

    // File set exactly as expected.
    assert_eq!(out.prompt_paths.len(), 2);
    assert_eq!(out.prompt_paths[0], dir.path().join("1_orchestrator.md"));
    assert_eq!(
        out.prompt_paths[1],
        dir.path().join("2_integrations_agent_gmail.md")
    );
    assert_eq!(out.summary_path, dir.path().join("SUMMARY.txt"));

    // Prompt body is raw bytes.
    let body = std::fs::read_to_string(&out.prompt_paths[0]).unwrap();
    assert_eq!(body, "# prompt for orchestrator\nbody\n");

    // Meta sidecar: exact byte format, toolkit-less variant.
    let meta0 = std::fs::read_to_string(dir.path().join("1_orchestrator.meta.txt")).unwrap();
    let expected_meta0 = "\
agent:          orchestrator
mode:           session
model:          claude-opus-4-7
workspace:      /tmp/ws
tool_count:     3
skill_tools:    1
";
    assert_eq!(meta0, expected_meta0);

    // Meta sidecar: toolkit variant inserts `toolkit:` after `agent:`.
    let meta1 =
        std::fs::read_to_string(dir.path().join("2_integrations_agent_gmail.meta.txt")).unwrap();
    let expected_meta1 = "\
agent:          integrations_agent
toolkit:        gmail
mode:           session
model:          claude-opus-4-7
workspace:      /tmp/ws
tool_count:     2
skill_tools:    1
";
    assert_eq!(meta1, expected_meta1);

    // SUMMARY.txt: one fixed-width row per dump.
    let summary = std::fs::read_to_string(&out.summary_path).unwrap();
    // Note: `{:<4}` pads the numeric fields, so rows carry three
    // trailing spaces. Preserved byte-for-byte from the pre-split
    // CLI implementation — any change here is an artefact-format
    // break.
    let expected_summary = "\
orchestrator                     tools=3    skill=1   \n\
integrations_agent@gmail         tools=2    skill=1   \n";
    assert_eq!(summary, expected_summary);
}

#[test]
fn sanitises_filename_components() {
    assert_eq!(sanitise_filename_component("gmail"), "gmail");
    assert_eq!(sanitise_filename_component("a/b c"), "a_b_c");
    assert_eq!(sanitise_filename_component("..-_ok"), "..-_ok");
    assert_eq!(sanitise_filename_component("weird:name*"), "weird_name_");
}
