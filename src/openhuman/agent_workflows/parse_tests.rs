//! Tests for WORKFLOW.md frontmatter parsing.

use super::*;

const FULL: &str = r#"---
name: bug-triage
description: How to handle an incoming bug report
when_to_use: a user reports a bug or something is broken
tags: [support, debugging]
tools:
  allow: [shell, git_operations]
phases:
  on_enter_directory:
    rules:
      - Read the local README before editing.
    context: [git]
  on_pick_up_task:
    rules:
      - Always reproduce before proposing a fix.
    scripts:
      - git fetch
    tools:
      allow: [shell, file_read]
      deny: [curl]
---
# Bug triage workflow

Body prose here.
"#;

#[test]
fn parses_full_frontmatter_and_body() {
    let (fm, body, warnings) = parse_workflow_md_str(FULL).expect("should parse");
    assert!(warnings.is_empty(), "warnings: {warnings:?}");
    assert_eq!(fm.name, "bug-triage");
    assert_eq!(
        fm.when_to_use,
        "a user reports a bug or something is broken"
    );
    assert_eq!(fm.tags, vec!["support", "debugging"]);
    assert_eq!(
        fm.tools.as_ref().unwrap().allow,
        vec!["shell", "git_operations"]
    );
    assert!(body.contains("Body prose here."));
}

#[test]
fn parses_phase_scripts_tools_and_context() {
    let (fm, _body, _w) = parse_workflow_md_str(FULL).unwrap();
    let pickup = fm.phases.get("on_pick_up_task").expect("phase present");
    assert_eq!(
        pickup.rules,
        vec!["Always reproduce before proposing a fix."]
    );
    assert_eq!(pickup.scripts, vec!["git fetch"]);
    let scope = pickup.tools.as_ref().unwrap();
    assert_eq!(scope.allow, vec!["shell", "file_read"]);
    assert_eq!(scope.deny, vec!["curl"]);

    let enter = fm.phases.get("on_enter_directory").unwrap();
    assert_eq!(enter.context, vec!["git"]);
}

#[test]
fn no_frontmatter_treats_whole_file_as_body() {
    let (fm, body, warnings) = parse_workflow_md_str("# Just a body\nno yaml").unwrap();
    assert_eq!(fm.name, "");
    assert!(body.contains("Just a body"));
    assert!(warnings.is_empty());
}

#[test]
fn unterminated_frontmatter_returns_none() {
    let content = "---\nname: x\nstill in frontmatter\n";
    assert!(parse_workflow_md_str(content).is_none());
}

#[test]
fn malformed_yaml_yields_warning_and_default_frontmatter() {
    // `phases` declared as a scalar instead of a map → deserialize error.
    let content = "---\nname: x\nphases: not-a-map\n---\nbody\n";
    let (fm, _body, warnings) = parse_workflow_md_str(content).unwrap();
    assert_eq!(fm.name, ""); // fell back to default on error
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("frontmatter parse error")),
        "warnings: {warnings:?}"
    );
}

#[test]
fn unknown_keys_land_in_extra() {
    let content = "---\nname: x\nfuture_key: 42\n---\nbody\n";
    let (fm, _body, _w) = parse_workflow_md_str(content).unwrap();
    assert!(fm.extra.contains_key("future_key"));
}
