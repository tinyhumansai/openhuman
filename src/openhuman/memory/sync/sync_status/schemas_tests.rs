use super::*;

#[test]
fn registers_only_status_list() {
    let regs = all_registered_controllers();
    assert_eq!(regs.len(), 1);
    assert_eq!(regs[0].schema.function, "status_list");
}

#[test]
fn schema_status_list_has_no_inputs_and_one_output() {
    let s = schemas("status_list");
    assert_eq!(s.namespace, "memory_sync");
    assert_eq!(s.function, "status_list");
    assert!(s.inputs.is_empty());
    assert_eq!(s.outputs.len(), 1);
    assert_eq!(s.outputs[0].name, "statuses");
}

#[test]
#[should_panic(expected = "unknown memory_sync schema function")]
fn schemas_panics_on_unknown_function() {
    schemas("nope");
}
