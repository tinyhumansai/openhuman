use super::*;

#[test]
fn schema_names_are_stable() {
    let s = team_schemas("team_list_members");
    assert_eq!(s.namespace, "team");
    assert_eq!(s.function, "list_members");
}

#[test]
fn controller_lists_match_lengths() {
    assert_eq!(
        all_team_controller_schemas().len(),
        all_team_registered_controllers().len()
    );
}

#[test]
fn schemas_match_unwrapped_team_payload_shapes() {
    let members = team_schemas("team_list_members");
    assert_eq!(members.outputs.len(), 1);
    assert_eq!(members.outputs[0].name, "result");
    assert_eq!(
        members.outputs[0].ty,
        TypeSchema::Array(Box::new(TypeSchema::Json))
    );

    let create_invite = team_schemas("team_create_invite");
    assert_eq!(create_invite.outputs.len(), 1);
    assert_eq!(create_invite.outputs[0].name, "result");
    assert_eq!(create_invite.outputs[0].ty, TypeSchema::Json);

    let invites = team_schemas("team_list_invites");
    assert_eq!(invites.outputs.len(), 1);
    assert_eq!(invites.outputs[0].name, "result");
    assert_eq!(
        invites.outputs[0].ty,
        TypeSchema::Array(Box::new(TypeSchema::Json))
    );
}
