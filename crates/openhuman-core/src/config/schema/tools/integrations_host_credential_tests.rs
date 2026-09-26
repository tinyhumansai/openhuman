use super::*;

/// Regression for CodeRabbit finding dca7d06a on PR #6689: pinning a
/// host credential must not leave a copy of its key sitting in a
/// normally-serialized `ComposioConfig` field. `host_credential` is
/// `#[serde(skip)]`; before this fix `pin_host_credential` also copied
/// the key into `api_key`, which is not skipped, so any config save
/// after pinning persisted the "runtime-only" secret to disk.
#[test]
fn pin_host_credential_does_not_leak_the_key_into_serialized_config() {
    let mut config = ComposioConfig::default();
    let credential = ComposioHostCredential::direct("sk-pinned-secret-do-not-serialize");
    config.pin_host_credential(credential);

    assert!(
        config.api_key.is_none(),
        "pinning must not populate the serialized api_key field"
    );

    let serialized = serde_json::to_string(&config).expect("ComposioConfig must serialize");
    assert!(
        !serialized.contains("sk-pinned-secret-do-not-serialize"),
        "{serialized}"
    );
    assert!(
        !serialized.contains("host_credential"),
        "host_credential is #[serde(skip)] and must never appear: {serialized}"
    );
}

/// The pinned-client factory path must still resolve the key from
/// `host_credential`, not from `api_key` — the field this fix stops
/// populating.
#[test]
fn pinned_client_still_resolves_the_key_from_host_credential() {
    let mut config = ComposioConfig::default();
    let credential = ComposioHostCredential::direct("sk-pinned-secret").entity_id("tenant-a");
    config.pin_host_credential(credential);

    let pinned = config.host_credential.as_ref().expect("credential pinned");
    assert_eq!(pinned.api_key(), "sk-pinned-secret");
    assert_eq!(pinned.entity(), "tenant-a");
}
