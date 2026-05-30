//! Tests for the workflow prompt catalog renderer.

use super::*;

fn wf(name: &str, dir: &str, when: &str) -> Workflow {
    Workflow {
        name: name.to_string(),
        dir_name: dir.to_string(),
        when_to_use: when.to_string(),
        ..Default::default()
    }
}

#[test]
fn empty_list_renders_empty_string() {
    assert_eq!(render_workflow_catalog(&[]), "");
    assert_eq!(render_available_workflows(&[]), "");
}

#[test]
fn renders_id_name_and_when_to_use() {
    let list = vec![wf("Bug Triage", "bug-triage", "a bug is reported")];
    let out = render_workflow_catalog(&list);
    assert!(out.contains("<available_workflows>"));
    assert!(out.contains("id=\"bug-triage\""));
    assert!(out.contains("name=\"Bug Triage\""));
    assert!(out.contains("a bug is reported"));
    assert!(out.contains("</available_workflows>"));
}

#[test]
fn escapes_xml_special_characters() {
    let list = vec![wf("A & B", "a-b", "use when <x> happens \"now\"")];
    let out = render_workflow_catalog(&list);
    assert!(out.contains("A &amp; B"));
    assert!(out.contains("&lt;x&gt;"));
    assert!(out.contains("&quot;now&quot;"));
}

#[test]
fn available_alias_matches_catalog() {
    let list = vec![wf("x", "x", "y")];
    assert_eq!(
        render_available_workflows(&list),
        render_workflow_catalog(&list)
    );
}
