//! Tests for phase guidance rendering, tool-scope resolution, and selection.

use super::*;
use crate::openhuman::agent_workflows::types::{
    WorkflowFrontmatter, WorkflowPhase, PHASE_CLOSE_TASK, PHASE_PICK_UP_TASK,
};
use std::collections::HashMap;

fn wf_with(name: &str, when_to_use: &str, phases: HashMap<String, WorkflowPhase>) -> Workflow {
    Workflow {
        name: name.to_string(),
        dir_name: name.to_string(),
        when_to_use: when_to_use.to_string(),
        phases: phases.clone(),
        frontmatter: WorkflowFrontmatter {
            name: name.to_string(),
            when_to_use: when_to_use.to_string(),
            phases,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn phase_guidance_renders_rules() {
    let mut phases = HashMap::new();
    phases.insert(
        PHASE_PICK_UP_TASK.to_string(),
        WorkflowPhase {
            rules: vec!["Reproduce first.".into(), "Write a test.".into()],
            ..Default::default()
        },
    );
    let wf = wf_with("bug-triage", "bug", phases);
    let out = phase_guidance(&wf, PHASE_PICK_UP_TASK).unwrap();
    assert!(out.contains("bug-triage"));
    assert!(out.contains("Reproduce first."));
    assert!(out.contains("Write a test."));
}

#[test]
fn phase_guidance_none_for_missing_or_empty_phase() {
    let wf = wf_with("x", "y", HashMap::new());
    assert!(phase_guidance(&wf, PHASE_CLOSE_TASK).is_none());

    let mut phases = HashMap::new();
    phases.insert(PHASE_CLOSE_TASK.to_string(), WorkflowPhase::default());
    let wf2 = wf_with("x", "y", phases);
    assert!(phase_guidance(&wf2, PHASE_CLOSE_TASK).is_none());
}

#[test]
fn effective_tool_scope_unions_phase_over_workflow_default() {
    let mut phases = HashMap::new();
    phases.insert(
        PHASE_PICK_UP_TASK.to_string(),
        WorkflowPhase {
            tools: Some(ToolScope {
                allow: vec!["file_read".into()],
                deny: vec!["curl".into()],
            }),
            ..Default::default()
        },
    );
    let mut wf = wf_with("x", "y", phases);
    wf.tools = Some(ToolScope {
        allow: vec!["shell".into(), "file_read".into()],
        deny: vec!["git_operations".into()],
    });

    let scope = effective_tool_scope(&wf, PHASE_PICK_UP_TASK).unwrap();
    // allow is the deduped union; deny is the union.
    assert!(scope.allow.contains(&"shell".to_string()));
    assert!(scope.allow.contains(&"file_read".to_string()));
    assert_eq!(
        scope.allow.iter().filter(|a| *a == "file_read").count(),
        1,
        "allow should be deduped"
    );
    assert!(scope.deny.contains(&"curl".to_string()));
    assert!(scope.deny.contains(&"git_operations".to_string()));
}

#[test]
fn effective_tool_scope_falls_back_to_workflow_or_phase_only() {
    // Workflow default only.
    let mut wf = wf_with("x", "y", HashMap::new());
    wf.tools = Some(ToolScope {
        allow: vec!["shell".into()],
        deny: vec![],
    });
    let scope = effective_tool_scope(&wf, PHASE_PICK_UP_TASK).unwrap();
    assert_eq!(scope.allow, vec!["shell"]);

    // Neither → None.
    let bare = wf_with("x", "y", HashMap::new());
    assert!(effective_tool_scope(&bare, PHASE_PICK_UP_TASK).is_none());
}

#[test]
fn best_match_scores_when_to_use() {
    let a = wf_with(
        "a",
        "a user reports a bug or something is broken",
        HashMap::new(),
    );
    let b = wf_with("b", "deploy the release to production", HashMap::new());
    let list = vec![a, b];
    let m = best_match(&list, "I think there is a bug, something broken").unwrap();
    assert_eq!(m.name, "a");
}

#[test]
fn best_match_none_when_nothing_overlaps() {
    let a = wf_with("a", "deploy release", HashMap::new());
    let list = vec![a];
    assert!(best_match(&list, "completely unrelated topic xyz").is_none());
}
