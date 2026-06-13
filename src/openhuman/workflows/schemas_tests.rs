use super::*;

#[test]
fn schema_names_are_stable() {
    let list = workflows_schemas("workflows_list");
    assert_eq!(list.namespace, "workflows");
    assert_eq!(list.function, "list");

    let read = workflows_schemas("workflows_read_resource");
    assert_eq!(read.namespace, "workflows");
    assert_eq!(read.function, "read_resource");
}

#[test]
fn controller_lists_match_lengths() {
    assert_eq!(
        all_workflows_controller_schemas().len(),
        all_workflows_registered_controllers().len()
    );
}

#[test]
fn skill_summary_round_trip_minimum_fields() {
    let skill = Workflow {
        name: "demo".to_string(),
        description: "desc".to_string(),
        version: "".to_string(),
        ..Default::default()
    };
    let summary: WorkflowSummary = skill.into();
    assert_eq!(summary.id, "demo");
    assert_eq!(summary.name, "demo");
    assert_eq!(summary.description, "desc");
}
