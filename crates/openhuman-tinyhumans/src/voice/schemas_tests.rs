use super::*;

#[test]
fn hosted_voice_controllers_keep_their_exact_rpc_names() {
    let names: Vec<String> = hosted_voice_controllers()
        .iter()
        .map(|c| format!("{}.{}", c.schema.namespace, c.schema.function))
        .collect();
    assert_eq!(names, ["voice.agent_signed_url", "voice.cloud_transcribe"]);
}

#[test]
fn cloud_transcribe_schema_requires_only_the_audio() {
    let schema = hosted_voice_schemas("voice_cloud_transcribe");
    let required: Vec<_> = schema
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(required, ["audio_base64"]);
    assert!(schema.inputs.iter().any(|f| f.name == "language"));
}

#[test]
fn agent_signed_url_takes_no_params() {
    assert!(hosted_voice_schemas("voice_agent_signed_url")
        .inputs
        .is_empty());
}

#[test]
fn registration_is_idempotent() {
    let _registry = crate::registry_test_lock();

    register();
    register();
}
