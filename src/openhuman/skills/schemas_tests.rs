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
    let skill = Skill {
        name: "demo".to_string(),
        description: "desc".to_string(),
        version: "".to_string(),
        ..Default::default()
    };
    let summary: SkillSummary = skill.into();
    assert_eq!(summary.id, "demo");
    assert_eq!(summary.name, "demo");
    assert_eq!(summary.description, "desc");
}
