use super::*;

#[test]
fn allowed_tools_accepts_yaml_sequence() {
    let fm: WorkflowFrontmatter =
        serde_yaml::from_str("allowed-tools:\n  - Bash\n  - Read\n").unwrap();
    assert_eq!(fm.allowed_tools, vec!["Bash", "Read"]);
}

#[test]
fn allowed_tools_accepts_comma_string() {
    // Claude Code convention: a scalar comma-joined string. Before the
    // string-or-seq deserializer this failed with "invalid type: string,
    // expected a sequence" and the skill's tools were silently dropped.
    let fm: WorkflowFrontmatter =
        serde_yaml::from_str("allowed-tools: Bash, Read, Grep, Skill, WebFetch").unwrap();
    assert_eq!(
        fm.allowed_tools,
        vec!["Bash", "Read", "Grep", "Skill", "WebFetch"]
    );
}

#[test]
fn allowed_tools_trims_whitespace_and_drops_empty_tokens() {
    // Hand-authored frontmatter is ragged: padding around each name and a
    // stray comma that yields an empty token. Both are dropped rather than
    // surfacing as a bogus tool name.
    let fm: WorkflowFrontmatter =
        serde_yaml::from_str(r#"allowed-tools: " Bash, , Read, ""#).unwrap();
    assert_eq!(fm.allowed_tools, vec!["Bash", "Read"]);
}

#[test]
fn allowed_tools_accepts_tools_alias() {
    let fm: WorkflowFrontmatter = serde_yaml::from_str("tools: Bash, Read").unwrap();
    assert_eq!(fm.allowed_tools, vec!["Bash", "Read"]);
}

#[test]
fn allowed_tools_accepts_snake_case_alias() {
    let fm: WorkflowFrontmatter = serde_yaml::from_str("allowed_tools: Bash, Read").unwrap();
    assert_eq!(fm.allowed_tools, vec!["Bash", "Read"]);
}

#[test]
fn allowed_tools_defaults_empty_when_absent() {
    let fm: WorkflowFrontmatter = serde_yaml::from_str("name: foo").unwrap();
    assert!(fm.allowed_tools.is_empty());
}
