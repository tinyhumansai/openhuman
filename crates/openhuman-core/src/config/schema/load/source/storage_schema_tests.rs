use super::*;
use serde_json::json;
use tinystoragedrivers::Meta;

#[test]
fn a_scopes_config_document_keeps_its_sealed_body_under_the_new_name() {
    let out = documents_v2(vec![SourceRecord {
        id: "u:alice/agent:x".into(),
        body: json!({ "toml_enc": "enc2:abcd" }),
        meta: Meta::unknown(),
    }])
    .unwrap();
    assert_eq!(out[0].collection, CONFIG_COLLECTION);
    assert_eq!(out[0].id, "u:alice%2Fagent:x");
    assert_eq!(out[0].body, json!({ "toml_enc": "enc2:abcd" }));
}

#[test]
fn the_schema_composes_under_its_owner() {
    tinystoragedrivers::schema::Registry::compose([storage_schema()]).unwrap();
}
