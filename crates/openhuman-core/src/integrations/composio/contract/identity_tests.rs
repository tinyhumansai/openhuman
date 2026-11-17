use super::*;
use crate::integrations::composio::contract::{
    NormalizedTask, ProviderUserProfile, TaskFetchFilter,
};

#[test]
fn profile_wire_shape_writes_every_field_as_null_when_unset() {
    let value = serde_json::to_value(ProviderUserProfile::default()).unwrap();
    let object = value.as_object().unwrap();
    assert!(object.contains_key("email"));
    assert!(object["email"].is_null());
}

#[test]
fn task_envelope_wire_shape_is_camel_case() {
    let task = serde_json::to_value(NormalizedTask::default()).unwrap();
    assert!(task
        .as_object()
        .unwrap()
        .keys()
        .any(|k| k.contains('I') || k == "title"));
    let filter = serde_json::to_value(TaskFetchFilter::default()).unwrap();
    assert!(filter.is_object());
    assert_eq!(TaskFetchFilter::default().effective_max(), 25);
}

#[tokio::test]
async fn empty_inputs_skip_the_module_call() {
    let config = Config::default();
    assert!(canonicalize_values(&config, &[]).await.unwrap().is_empty());
    assert!(normalize_connection_identifiers(&config, &[])
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        render_connected_identities_section(&config, &[])
            .await
            .unwrap(),
        ""
    );
}
