use super::*;
use crate::core::runtime::{CoreContext, DomainSet, SaasConfig};

fn host(tmp: &tempfile::TempDir) -> ProfileHost {
    ProfileHost::new(
        SaasConfig::new(tmp.path()),
        CoreContext::for_test(DomainSet::full(), None),
    )
}

#[tokio::test]
async fn provision_derives_the_profile_and_never_echoes_the_user_id() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp);
    let out = provision_on(&host, "alice@example.com").await.unwrap();
    let json = out.into_cli_compatible_json().unwrap().to_string();
    assert!(!json.contains("alice"), "{json}");
    assert!(json.contains("\"created\":true"), "{json}");
    let again = provision_on(&host, "alice@example.com").await.unwrap();
    assert!(again
        .into_cli_compatible_json()
        .unwrap()
        .to_string()
        .contains("\"created\":false"));
}

#[tokio::test]
async fn status_and_deprovision_take_profile_ids_only() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp);
    assert!(status_on(&host, "alice@example.com").await.is_err());
    assert!(deprovision_on(&host, "../operator").await.is_err());

    let id = ProfileId::for_user("alice", crate::profiles::ProfileIdMode::Raw).unwrap();
    assert!(status_on(&host, id.as_str())
        .await
        .unwrap_err()
        .contains("not provisioned"));
    provision_on(&host, "alice").await.unwrap();
    status_on(&host, id.as_str()).await.unwrap();
    deprovision_on(&host, id.as_str()).await.unwrap();
    assert!(status_on(&host, id.as_str()).await.is_err());
}

#[tokio::test]
async fn credentials_are_set_and_cleared_per_profile_and_never_echoed() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp);
    // The keyring holding credential secrets is shared by every test here.
    let alice_user = format!("alice-{}", uuid::Uuid::new_v4());
    let bob_user = format!("bob-{}", uuid::Uuid::new_v4());
    let alice = ProfileId::for_user(&alice_user, crate::profiles::ProfileIdMode::Raw).unwrap();
    let bob = ProfileId::for_user(&bob_user, crate::profiles::ProfileIdMode::Raw).unwrap();
    assert!(set_credential_on(
        &host,
        alice.as_str(),
        UserCredentialKind::Session,
        "t",
        None
    )
    .await
    .unwrap_err()
    .contains("not provisioned"));
    provision_on(&host, &alice_user).await.unwrap();
    provision_on(&host, &bob_user).await.unwrap();

    let out = set_credential_on(
        &host,
        alice.as_str(),
        UserCredentialKind::Session,
        "alice-secret-jwt",
        None,
    )
    .await
    .unwrap();
    let json = out.into_cli_compatible_json().unwrap().to_string();
    assert!(!json.contains("alice-secret-jwt"), "{json}");
    assert!(host.summary(&alice).await.unwrap().unwrap().has_credential);
    assert!(!host.summary(&bob).await.unwrap().unwrap().has_credential);

    clear_credential_on(&host, alice.as_str()).await.unwrap();
    assert!(!host.summary(&alice).await.unwrap().unwrap().has_credential);
}

/// Keyring secrets are namespaced by their store's directory name, so a
/// profile named like a configured operator directory would share the
/// operator's credential slots.
#[tokio::test]
async fn a_profile_named_like_the_operator_dir_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let mut saas = SaasConfig::new(tmp.path());
    saas.operator_dir = Some(tmp.path().join("node-a"));
    let host = ProfileHost::new(saas, CoreContext::for_test(DomainSet::full(), None));

    let err = provision_on(&host, "node-a").await.unwrap_err();
    assert!(err.contains("reserved"), "{err}");
    assert!(!tmp.path().join("users").join("node-a").exists());
    provision_on(&host, "node-b").await.unwrap();
}
