//! Which secret scope may use the admitted TinySecurity module.

use super::*;

#[test]
fn the_admitting_scope_keeps_its_module() {
    let path = Path::new("/home/a/.openhuman/.secret_key");
    assert_eq!(claim_scope(Some(path), path), Ok(ScopeClaim::Held));
}

#[test]
fn a_different_scope_is_refused_while_the_owner_is_live() {
    let owner = Path::new("/home/a/.openhuman/.secret_key");
    let other = Path::new("/home/b/.openhuman/.secret_key");
    assert_eq!(
        claim_scope(Some(owner), other),
        Err("security secret scope changed after module admission".to_owned())
    );
}

#[test]
fn a_scope_takes_over_a_module_whose_owner_was_released() {
    let other = Path::new("/tmp/openhuman-scratch-second/.secret_key");
    assert_eq!(claim_scope(None, other), Ok(ScopeClaim::TakenOver));
}

#[test]
fn releasing_the_owner_lets_the_next_scope_claim_and_then_hold_the_module() {
    let id = "tinysecurity-claim-test";
    let first = PathBuf::from("/tmp/openhuman-scratch-first/.secret_key");
    let second = PathBuf::from("/tmp/openhuman-scratch-second/.secret_key");
    set_scope_owner(id, first.clone());
    assert!(claim_scope(scope_owner(id).as_deref(), &second).is_err());

    release_scope_owner(id);
    assert_eq!(scope_owner(id), None);
    assert_eq!(
        claim_scope(scope_owner(id).as_deref(), &second),
        Ok(ScopeClaim::TakenOver)
    );
    set_scope_owner(id, second.clone());

    assert_eq!(
        claim_scope(scope_owner(id).as_deref(), &second),
        Ok(ScopeClaim::Held)
    );
    assert!(
        claim_scope(scope_owner(id).as_deref(), &first).is_err(),
        "the released scope cannot return while the new owner is live"
    );
    release_scope_owner(id);
}
