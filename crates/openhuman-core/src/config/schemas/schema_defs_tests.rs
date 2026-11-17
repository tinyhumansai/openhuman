use super::*;

#[test]
fn cloud_provider_schema_documents_ca_certificate_field() {
    let schema = schemas("update_model_settings");
    let providers = schema
        .inputs
        .iter()
        .find(|field| field.name == "cloud_providers")
        .expect("cloud_providers schema input");

    assert!(providers.comment.contains("ca_cert_pem"));
}
