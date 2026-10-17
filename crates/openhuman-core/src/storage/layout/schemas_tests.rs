use super::*;

#[test]
fn the_three_controllers_are_registered_under_storage() {
    let names: Vec<_> = all_registered_controllers()
        .iter()
        .map(|c| (c.schema.namespace, c.schema.function))
        .collect();
    assert_eq!(
        names,
        vec![
            ("storage", "migrate"),
            ("storage", "status"),
            ("storage", "restore")
        ]
    );
    assert_eq!(all_controller_schemas().len(), 3);
    assert!(schemas("restore")
        .inputs
        .iter()
        .any(|f| f.name == "archive" && f.required));
    assert!(schemas("migrate")
        .inputs
        .iter()
        .any(|f| f.name == "dry_run" && !f.required));
    assert_eq!(schemas("nope").function, "unknown");
}
