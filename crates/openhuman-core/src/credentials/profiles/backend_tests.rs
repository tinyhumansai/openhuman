use super::*;
use std::path::PathBuf;

fn users() -> PathBuf {
    PathBuf::from("/srv/saas/users")
}

#[test]
fn without_a_backend_every_store_stays_local() {
    let alice = users().join("alice");
    assert_eq!(
        decide(false, Some("alice"), &alice, Some(&users())),
        Home::Local
    );
}

#[test]
fn a_tenant_store_under_its_own_scope_lives_on_the_backend() {
    let alice = users().join("alice");
    assert_eq!(
        decide(true, Some("alice"), &alice, Some(&users())),
        Home::Tenant("alice".into())
    );
    // An in-process host without a registered SaaS root (embedding, tests)
    // recognises the same `users/<id>` shape.
    assert_eq!(
        decide(true, Some("alice"), &alice, None),
        Home::Tenant("alice".into())
    );
}

#[test]
fn a_tenant_store_outside_its_own_scope_is_refused_in_saas() {
    let alice = users().join("alice");
    assert_eq!(
        decide(true, Some("bob"), &alice, Some(&users())),
        Home::Refused,
        "another tenant's scope"
    );
    assert_eq!(
        decide(true, None, &alice, Some(&users())),
        Home::Refused,
        "the operator plane or an unscoped task"
    );
    // A same-named directory outside the SaaS root is not a tenant store.
    assert_eq!(
        decide(
            true,
            Some("alice"),
            Path::new("/elsewhere/users/alice"),
            Some(&users())
        ),
        Home::Local
    );
}

#[test]
fn desktop_and_operator_stores_keep_their_local_file() {
    // The desktop's `users/<id>` with its default storage backend: no tenant
    // context and no SaaS host.
    assert_eq!(
        decide(true, None, Path::new("/home/u/.openhuman/users/abc"), None),
        Home::Local
    );
    // The SaaS operator plane's own store, outside `<root>/users`.
    assert_eq!(
        decide(true, None, Path::new("/srv/saas/operator"), Some(&users())),
        Home::Local
    );
}
