use super::*;

#[test]
fn all_controller_schemas_and_registered_controllers_stay_in_sync() {
    let schemas = all_controller_schemas();
    let controllers = all_registered_controllers();
    assert_eq!(schemas.len(), controllers.len());
    assert!(schemas.iter().all(|s| s.namespace == NAMESPACE));
}

#[test]
#[should_panic(expected = "unknown memory_sources schema function")]
fn schemas_panics_on_unknown_function() {
    schemas("nope");
}
