use super::*;

#[test]
fn config_collection_has_a_valid_owner_declaration() {
    let specs = collection_specs();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].name, CONFIG_COLLECTION);
    specs[0].validate().unwrap();
}

#[test]
fn config_document_ids_remain_scope_derived() {
    assert_eq!(document_id("u:alice/agent:x").unwrap(), "u:alice%2Fagent:x");
}
