//! Which secret scope may use the admitted TinySecurity module.

use super::*;

const CHANGED: &str = "security secret scope changed after module admission";

fn admitted() -> &'static Path {
    Path::new("/tmp/openhuman-scratch-first/.secret_key")
}

#[test]
fn the_owning_scope_keeps_its_module() {
    assert_eq!(
        claim_scope(Some(admitted()), admitted(), admitted(), false),
        Ok(ScopeClaim::Held)
    );
}

#[test]
fn a_different_scope_is_refused_while_the_owner_is_live() {
    let other = Path::new("/tmp/openhuman-scratch-second/.secret_key");
    assert_eq!(
        claim_scope(Some(admitted()), admitted(), other, true),
        Err(CHANGED.to_owned()),
        "even a transient scope waits for the owner's release"
    );
}

#[test]
fn a_transient_scope_takes_over_a_module_whose_owner_was_released() {
    let other = Path::new("/tmp/openhuman-scratch-second/.secret_key");
    assert_eq!(
        claim_scope(None, admitted(), other, true),
        Ok(ScopeClaim::TakenOver)
    );
}

#[test]
fn the_admitting_scope_takes_its_module_back_after_a_release() {
    assert_eq!(
        claim_scope(None, admitted(), admitted(), false),
        Ok(ScopeClaim::TakenOver)
    );
}

#[test]
fn a_durable_scope_is_refused_a_module_whose_owner_was_released() {
    let durable = Path::new("/home/someone/.openhuman/.secret_key");
    let refused = claim_scope(None, admitted(), durable, false).expect_err("durable refused");
    assert!(refused.starts_with(CHANGED), "{refused}");
    assert!(refused.contains("durable"), "{refused}");
}

/// The ownership table is process-wide, so its lifecycle is one test.
#[test]
fn release_declare_and_claim_follow_the_hosts_lifecycle() {
    let id = "tinysecurity-ownership-test";
    let first_dir = Path::new("/tmp/openhuman-scratch-first");
    let second_dir = Path::new("/tmp/openhuman-scratch-second");
    let first = secret_key_path(first_dir);
    let second = secret_key_path(second_dir);

    // The first runtime admits the module.
    assert!(publish_owner(id, first.clone(), scope_epoch()));
    assert_eq!(claim(id, &first, &first), Ok(ScopeClaim::Held));

    // A second runtime is refused while the first is live, declared or not.
    declare_transient_scope(second_dir);
    assert_eq!(claim(id, &first, &second), Err(CHANGED.to_owned()));

    // A load the first runtime started before its release finishes after it:
    // it must not record the released scope as the owner.
    let stale_epoch = scope_epoch();
    release_scopes();
    assert!(!publish_owner(id, first.clone(), stale_epoch));

    // The release also forgot the second's declaration: it must declare again.
    assert!(claim(id, &first, &second).is_err());
    declare_transient_scope(second_dir);
    assert_eq!(claim(id, &first, &second), Ok(ScopeClaim::TakenOver));
    assert!(publish_owner(id, second.clone(), scope_epoch()));

    // Now the second owns it, and the released first is refused.
    assert_eq!(claim(id, &first, &second), Ok(ScopeClaim::Held));
    assert_eq!(claim(id, &first, &first), Err(CHANGED.to_owned()));
    release_scopes();
}
