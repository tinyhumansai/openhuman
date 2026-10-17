use super::*;
use crate::storage::MemoryStorage;

fn stores() -> TenantSessionStores {
    TenantSessionStores::new(Arc::new(MemoryStorage::new()), false).expect("bridge")
}

#[test]
fn an_unprofiled_key_routes_to_the_local_tenant() {
    let router = stores();
    let (provider, agent) = router.route("asha");
    assert_eq!(provider.tenant(), &Scope::local());
    assert_eq!(agent, "asha");
    assert_eq!(
        provider.scope_for(&agent).unwrap().as_str(),
        "local/agent:asha"
    );
}

#[test]
fn a_profile_key_routes_to_the_profile_tenant() {
    let router = stores();
    let key = crate::core::runtime::session_key(&crate::core::runtime::Tenant {
        profile: Some("alice".into()),
        agent: Some("asha".into()),
    });
    let (provider, agent) = router.route(&key);
    assert_eq!(provider.tenant().as_str(), "u:alice");
    assert_eq!(
        provider.scope_for(&agent).unwrap().as_str(),
        "u:alice/agent:asha"
    );
}

#[test]
fn escaped_keys_round_trip_to_the_same_tenant_and_agent() {
    let router = stores();
    let key = crate::core::runtime::session_key(&crate::core::runtime::Tenant {
        profile: Some("a~b%c".into()),
        agent: None,
    });
    let (provider, agent) = router.route(&key);
    assert_eq!(
        provider.tenant(),
        &crate::storage::scope_for_profile("a~b%c")
    );
    assert_eq!(agent, "default");
}

#[test]
fn one_provider_serves_every_agent_of_a_tenant() {
    let router = stores();
    let (first, _) = router.route("one");
    let (second, _) = router.route("two");
    assert!(Arc::ptr_eq(&first, &second));
    let (other, _) = router.route("u~two");
    assert!(!Arc::ptr_eq(&first, &other));
}
