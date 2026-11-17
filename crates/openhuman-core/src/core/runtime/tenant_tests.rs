use super::*;

use crate::core::runtime::{ContextOverlay, DomainSet};
use crate::tools::toolpacks::ToolGroups;

fn tenant(profile: Option<&str>, agent: Option<&str>) -> Tenant {
    Tenant {
        profile: profile.map(str::to_owned),
        agent: agent.map(str::to_owned),
    }
}

fn overlay() -> ContextOverlay {
    ContextOverlay::new(
        crate::config::Config::default(),
        DomainSet::kernel(),
        ToolGroups::none(),
    )
}

#[test]
fn a_saas_task_without_scope_has_no_tenant() {
    assert_eq!(tenant_in(true, None), Err(NoTenant));
    assert_eq!(tenant_in(false, None), Ok(Tenant::default()));
}

#[test]
fn the_tenant_is_what_the_context_carries() {
    let root = CoreContext::for_test(DomainSet::full(), None);
    assert_eq!(tenant_in(true, Some(&root)), Ok(Tenant::default()));

    let user = root.derive_with(overlay().profile("u-alice").session_agent("u-alice"));
    let got = tenant_in(true, Some(&user)).unwrap();
    assert_eq!(got, tenant(Some("u-alice"), Some("u-alice")));
    assert_eq!(got.profile_only(), tenant(Some("u-alice"), None));
}

#[test]
fn a_derived_context_inherits_the_profile_and_a_profile_gets_fresh_slots() {
    let root = CoreContext::for_test(DomainSet::full(), None);
    let profile = root.derive_with(overlay().profile("p1"));
    assert_eq!(profile.profile(), Some("p1"));
    assert_eq!(profile.session_agent(), None);

    // A plain derive (a turn context) keeps the profile and its slots.
    let turn = profile.derive_with(overlay());
    assert_eq!(turn.profile(), Some("p1"));
    *turn
        .agent_state()
        .slot::<std::sync::Mutex<u32>>()
        .lock()
        .unwrap() = 7;
    assert_eq!(
        *profile
            .agent_state()
            .slot::<std::sync::Mutex<u32>>()
            .lock()
            .unwrap(),
        7,
        "a turn context shares its profile's slots"
    );

    // Another profile never does.
    let other = root.derive_with(overlay().profile("p2"));
    assert_eq!(
        *other
            .agent_state()
            .slot::<std::sync::Mutex<u32>>()
            .lock()
            .unwrap(),
        0
    );
    assert_eq!(
        *root
            .agent_state()
            .slot::<std::sync::Mutex<u32>>()
            .lock()
            .unwrap(),
        0,
        "the operator's slots are not the profile's"
    );
}

#[test]
fn a_tenant_is_in_use_while_a_turn_context_or_a_handle_lives() {
    let root = CoreContext::for_test(DomainSet::full(), None);
    let profile = root.derive_with(overlay().profile("p1"));
    assert!(!profile.tenant_in_use());

    let turn = profile.derive_with(overlay());
    assert!(
        profile.tenant_in_use(),
        "a live turn context shares the slots"
    );
    drop(turn);
    assert!(!profile.tenant_in_use());

    let handle = Arc::clone(&profile);
    assert!(
        profile.tenant_in_use(),
        "a second handle (a request, a task)"
    );
    drop(handle);
    assert!(!profile.tenant_in_use());

    // A derived profile of its own does not count against the parent.
    let _child = profile.derive_with(overlay().profile("p2"));
    assert!(!profile.tenant_in_use());
}

#[tokio::test]
async fn a_turn_scope_keeps_the_tenant_in_use() {
    let root = CoreContext::for_test(DomainSet::full(), None);
    let profile = root.derive_with(overlay().profile("p1"));
    let probe = &profile;
    let (in_turn, tenant) =
        CoreContext::scope_with_turn_origin(Arc::clone(&profile), None, async move {
            (probe.tenant_in_use(), current_tenant_or_isolated("test"))
        })
        .await;
    assert!(in_turn, "a running turn holds its profile");
    assert_eq!(tenant.profile.as_deref(), Some("p1"));
    assert!(!profile.tenant_in_use(), "released once the turn ends");
}

#[test]
fn desktop_keys_are_unchanged() {
    let desktop = Tenant::default();
    assert_eq!(tenant_key(&desktop, "t1"), "t1");
    assert_eq!(tenant_key(&desktop, ""), "");
    // The embedded-agent form is the one web_chat used before profiles.
    assert_eq!(
        tenant_key(&tenant(None, Some("agent")), "t1"),
        "\u{1f}5:agentt1"
    );
    assert_eq!(session_key(&desktop), "default");
    assert_eq!(session_key(&tenant(None, Some("a1"))), "a1");
}

#[test]
fn profiles_prefix_their_keys() {
    let alice = tenant(Some("u-alice"), None);
    let bob = tenant(Some("u-bob"), None);
    assert_ne!(tenant_key(&alice, "t1"), tenant_key(&bob, "t1"));
    assert_ne!(tenant_key(&alice, "t1"), "t1");
    assert_eq!(session_key(&alice), "u-alice~default");
    assert_eq!(session_key(&tenant(Some("u-a"), Some("x"))), "u-a~x");
    assert_eq!(session_key(&tenant(Some("a~b"), Some("c"))), "a%7Eb~c");
    assert_ne!(
        session_key(&tenant(Some("a~b"), Some("c"))),
        session_key(&tenant(Some("a"), Some("b~c")))
    );
}

#[test]
fn a_profiles_session_keys_share_its_prefix_and_no_other_does() {
    let prefix = session_key_prefix("alice");
    assert_eq!(prefix, "alice~");
    assert!(session_key(&tenant(Some("alice"), None)).starts_with(&prefix));
    assert!(session_key(&tenant(Some("alice"), Some("sub-1"))).starts_with(&prefix));
    assert!(!session_key(&tenant(Some("alice2"), None)).starts_with(&prefix));
    // An unprofiled agent's `~` is escaped, so it cannot pose as a profile.
    assert!(!session_key(&tenant(None, Some("alice~x"))).starts_with(&prefix));
    assert_eq!(session_key_prefix("a~b"), "a%7Eb~");
}

#[test]
fn an_unprofiled_agent_never_collides_with_a_profiled_one() {
    assert_ne!(
        session_key(&tenant(None, Some("a~b"))),
        session_key(&tenant(Some("a"), Some("b")))
    );
    assert_eq!(session_key(&tenant(None, Some("plain"))), "plain");
    assert_ne!(
        session_key(&tenant(None, Some("a~b"))),
        session_key(&tenant(None, Some("a%7Eb")))
    );
}

#[test]
fn keys_split_back_into_their_parts() {
    for (t, id) in [
        (tenant(None, None), "t1"),
        (tenant(None, None), "\u{1f}x"),
        (tenant(None, None), "\u{1e}3:abc"),
        (tenant(None, Some("ag:1")), "t1"),
        (tenant(Some("p"), None), "\u{1f}2:x"),
        (tenant(Some("p:9"), Some("a")), ""),
    ] {
        let key = tenant_key(&t, id);
        assert_eq!(split_key(&key), Some((t.clone(), id)), "{key:?}");
        assert_eq!(id_of_key(&key), Some(id));
    }
    assert_eq!(split_key("\u{1e}x"), None);
    assert_eq!(split_key("\u{1e}9:ab"), None);
}

#[tokio::test]
async fn profile_keys_drop_the_agent() {
    let root = CoreContext::for_test(DomainSet::full(), None);
    let user = root.derive_with(overlay().profile("p1").session_agent("a1"));
    let key = profile_key_in(&Tenant::of(&user), "t1");
    assert_eq!(key, tenant_key(&tenant(Some("p1"), None), "t1"));
    let agent_only = root.derive_with(overlay().session_agent("a1"));
    let key = profile_key_in(&Tenant::of(&agent_only), "t1");
    assert_eq!(
        key, "t1",
        "no profile: the bare id, embedded agents included"
    );
}

#[test]
fn only_the_unprofiled_default_orchestrator_is_unscoped() {
    assert!(!tenant(None, None).is_scoped());
    assert!(tenant(Some("alice"), None).is_scoped());
    assert!(tenant(None, Some("alpha")).is_scoped());
    assert!(tenant(Some("alice"), Some("alpha")).is_scoped());
}
