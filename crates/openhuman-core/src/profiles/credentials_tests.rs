use super::*;
use crate::credentials::backend_credential::{resolve_backend_credential, BackendCredential};
use crate::profiles::layout::{profile_config, ProfileLayout};
use crate::profiles::ProfileId;

/// Credential secrets live in the process keyring keyed by agent id, which
/// every test in this binary shares; each test therefore uses its own users.
fn profile(tmp: &tempfile::TempDir, user: &str) -> Config {
    let id = ProfileId::for_user(
        &format!("{user}-{}", uuid::Uuid::new_v4()),
        crate::profiles::ProfileIdMode::Raw,
    )
    .unwrap();
    let layout = ProfileLayout::new(tmp.path(), &id);
    std::fs::create_dir_all(&layout.workspace_dir).unwrap();
    profile_config(&layout, &id)
}

#[tokio::test]
async fn each_profile_resolves_only_its_own_credential() {
    let tmp = tempfile::tempdir().unwrap();
    let alice = profile(&tmp, "alice");
    let bob = profile(&tmp, "bob");
    store(&alice, UserCredentialKind::Session, "alice-jwt", None)
        .await
        .unwrap();
    store(&bob, UserCredentialKind::ApiKey, "bob-key", None)
        .await
        .unwrap();

    assert_eq!(
        resolve_backend_credential(&alice).await.unwrap(),
        BackendCredential::Session("alice-jwt".into())
    );
    assert_eq!(
        resolve_backend_credential(&bob).await.unwrap(),
        BackendCredential::ApiKey("bob-key".into())
    );
}

#[tokio::test]
async fn clearing_one_profile_leaves_the_other() {
    let tmp = tempfile::tempdir().unwrap();
    let alice = profile(&tmp, "alice");
    let bob = profile(&tmp, "bob");
    store(&alice, UserCredentialKind::Session, "alice-jwt", None)
        .await
        .unwrap();
    store(&bob, UserCredentialKind::Session, "bob-jwt", None)
        .await
        .unwrap();
    assert!(clear(&alice).await.unwrap());
    assert!(!has(&alice).await);
    assert!(has(&bob).await);
    assert!(!clear(&alice).await.unwrap(), "nothing left to clear");
}

#[tokio::test]
async fn an_expired_session_is_rejected_locally() {
    let tmp = tempfile::tempdir().unwrap();
    let alice = profile(&tmp, "alice");
    store(
        &alice,
        UserCredentialKind::Session,
        "alice-jwt",
        Some("2000-01-01T00:00:00Z"),
    )
    .await
    .unwrap();
    let err = resolve_backend_credential(&alice).await.unwrap_err();
    assert!(err.contains("SESSION_EXPIRED"), "{err}");
}

#[tokio::test]
async fn bad_input_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let alice = profile(&tmp, "alice");
    assert!(store(&alice, UserCredentialKind::Session, "  ", None)
        .await
        .is_err());
    assert!(
        store(&alice, UserCredentialKind::Session, "t", Some("tomorrow"))
            .await
            .is_err()
    );
    assert!(!has(&alice).await);
}

#[tokio::test]
async fn installing_one_kind_replaces_the_other() {
    let tmp = tempfile::tempdir().unwrap();
    let alice = profile(&tmp, "alice-rotates");
    store(&alice, UserCredentialKind::ApiKey, "old-key", None)
        .await
        .unwrap();
    store(&alice, UserCredentialKind::Session, "new-jwt", None)
        .await
        .unwrap();
    assert_eq!(
        resolve_backend_credential(&alice).await.unwrap(),
        BackendCredential::Session("new-jwt".into()),
        "the old API key no longer wins"
    );
    store(&alice, UserCredentialKind::ApiKey, "newer-key", None)
        .await
        .unwrap();
    assert_eq!(
        resolve_backend_credential(&alice).await.unwrap(),
        BackendCredential::ApiKey("newer-key".into())
    );
    assert!(clear(&alice).await.unwrap());
}

#[tokio::test]
async fn every_profile_of_the_other_kind_is_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let alice = profile(&tmp, "alice-profiles");
    // A non-default, active API-key profile.
    AuthService::from_config(&alice)
        .store_provider_token(
            api_key::API_KEY_PROVIDER,
            "other",
            "side-key",
            HashMap::new(),
            true,
        )
        .await
        .unwrap();
    store(&alice, UserCredentialKind::Session, "jwt", None)
        .await
        .unwrap();
    assert_eq!(
        resolve_backend_credential(&alice).await.unwrap(),
        BackendCredential::Session("jwt".into())
    );
    assert!(clear(&alice).await.unwrap());
    assert!(!has(&alice).await);
}

#[tokio::test]
async fn concurrent_stores_of_different_kinds_leave_exactly_one_kind() {
    let tmp = tempfile::tempdir().unwrap();
    for round in 0..8 {
        let user = profile(&tmp, &format!("race{round}"));
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let spawn = |kind, token: &'static str| {
            let (user, barrier) = (user.clone(), barrier.clone());
            tokio::spawn(async move {
                barrier.wait().await;
                store(&user, kind, token, None).await.unwrap();
            })
        };
        let session = spawn(UserCredentialKind::Session, "race-jwt");
        let key = spawn(UserCredentialKind::ApiKey, "race-key");
        session.await.unwrap();
        key.await.unwrap();

        // Whichever store ran last owns the profile; the other kind is gone.
        let auth = AuthService::from_config(&user);
        let profiles = auth.load_profiles().await.unwrap().profiles;
        let has_key = profiles.values().any(|profile| {
            profile.provider
                == crate::credentials::normalize_provider(api_key::API_KEY_PROVIDER).unwrap()
        });
        let has_session = profiles.values().any(|profile| {
            profile.provider
                == crate::credentials::normalize_provider(APP_SESSION_PROVIDER).unwrap()
        });
        assert!(
            has_key != has_session,
            "round {round}: exactly one kind must remain (key={has_key}, session={has_session})"
        );
    }
}
