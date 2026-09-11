use super::*;

#[test]
fn schema_names_are_stable() {
    let list = skills_schemas("skills_list");
    assert_eq!(list.namespace, "skills");
    assert_eq!(list.function, "list");

    let read = skills_schemas("skills_read_resource");
    assert_eq!(read.namespace, "skills");
    assert_eq!(read.function, "read_resource");
}

#[test]
fn controller_lists_match_lengths() {
    assert_eq!(
        all_skills_controller_schemas().len(),
        all_skills_registered_controllers().len()
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

/// Pins the declared controller-schema output names to the workflow-spelled
/// wire reality. The skills→workflows rename left two schemas emitting the old
/// spelling (`new_skills` / `skill`) while the handlers and wire structs
/// (`WorkflowsInstallFromUrlResult.new_workflows`, `WorkflowsCreateResult.workflow`)
/// switched to the new one — a silent schema/wire divergence the frontend
/// papered over by reading the wire names directly. These assertions fail on
/// the pre-rename schema and stop it from drifting back.
#[test]
fn install_and_create_schemas_declare_workflow_output_names() {
    // Arrange
    let install = skills_schemas("skills_install_from_url");
    let create = skills_schemas("skills_create");
    let update = skills_schemas("skills_update");

    // Act
    let has_output = |schema: &crate::core::ControllerSchema, name: &str| {
        schema.outputs.iter().any(|f| f.name == name)
    };

    // Assert — install echoes the workflow-spelled slug list, never the old one.
    assert!(
        has_output(&install, "new_workflows"),
        "skills_install_from_url must declare `new_workflows`; outputs: {:?}",
        install.outputs.iter().map(|f| f.name).collect::<Vec<_>>()
    );
    assert!(
        !has_output(&install, "new_skills"),
        "skills_install_from_url must not declare the pre-rename `new_skills`"
    );

    // create / update share one schema; both carry the `workflow` output.
    for (function, schema) in [("skills_create", &create), ("skills_update", &update)] {
        assert!(
            has_output(schema, "workflow"),
            "{function} must declare `workflow`; outputs: {:?}",
            schema.outputs.iter().map(|f| f.name).collect::<Vec<_>>()
        );
        assert!(
            !has_output(schema, "skill"),
            "{function} must not declare the pre-rename `skill`"
        );
    }
}
