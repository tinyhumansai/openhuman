use super::*;

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_listed_whole_tool_grant_can_be_revoked_by_its_opaque_id() {
    let world = world().await;
    let first = crate::core::runtime::Tenant::default();
    let second = crate::core::runtime::Tenant {
        profile: Some("another-profile".into()),
        agent: Some("another-agent".into()),
    };

    let shown = Arc::new(Shown::default());
    let mut first_check = input("gmail_send", &shown);
    first_check.tenant = first.clone();
    world.engine.check(first_check).await.unwrap();
    let mut second_check = input("gmail_send", &shown);
    second_check.tenant = second.clone();
    world.engine.check(second_check).await.unwrap();

    world
        .engine
        .grant_tool_trust(&first, Variant::Standard, "gmail_send")
        .await
        .unwrap();
    world
        .engine
        .grant_tool_trust(&second, Variant::Standard, "gmail_send")
        .await
        .unwrap();

    let first_grants = world
        .engine
        .tool_trust(&first, Variant::Standard, Posture::default())
        .await
        .unwrap();
    assert_eq!(first_grants.len(), 1);
    let request_id = first_grants[0].request_id.clone();
    assert!(!request_id.is_empty());

    assert!(!world
        .engine
        .revoke_tool_trust_by_id(&second, Variant::Standard, Posture::default(), &request_id)
        .await
        .unwrap());
    assert_eq!(
        world
            .engine
            .tool_trust(&second, Variant::Standard, Posture::default())
            .await
            .unwrap()
            .len(),
        1,
        "an ID from another registered scope must not revoke this grant"
    );

    assert!(world
        .engine
        .revoke_tool_trust_by_id(&first, Variant::Standard, Posture::default(), &request_id)
        .await
        .unwrap());
    assert!(!world
        .engine
        .revoke_tool_trust_by_id(&first, Variant::Standard, Posture::default(), &request_id)
        .await
        .unwrap());
    assert!(world
        .engine
        .tool_trust(&first, Variant::Standard, Posture::default())
        .await
        .unwrap()
        .is_empty());

    let shown = Arc::new(Shown::default());
    assert!(matches!(
        world
            .engine
            .check(input("gmail_send", &shown))
            .await
            .unwrap(),
        Checked::Pending { .. }
    ));
}
