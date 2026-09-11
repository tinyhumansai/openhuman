use super::*;

#[test]
fn sync_schema_exposes_all_functions() {
    assert_eq!(
        FUNCTIONS,
        &[
            "sync_channel",
            "sync_all",
            "ingestion_status",
            "scheduler_override"
        ]
    );
    assert_eq!(controllers().len(), FUNCTIONS.len());
}

#[test]
fn unknown_sync_schema_returns_none() {
    assert!(schema("not_real").is_none());
}

#[test]
fn sync_channel_schema_requires_channel_id() {
    let schema = schema("sync_channel").unwrap();
    assert_eq!(schema.inputs.len(), 1);
    assert_eq!(schema.inputs[0].name, "channel_id");
    assert!(schema.inputs[0].required);
}
