use super::*;

fn failing() -> tinybus::Result<LinkedModule> {
    Err(tinybus::Error::failed("test factory"))
}

#[test]
fn registration_is_visible_and_idempotent() {
    assert!(!is_registered("linked-registry-test"));
    register_linked("linked-registry-test", failing);
    register_linked("linked-registry-test", failing);
    assert!(is_registered("linked-registry-test"));
    assert_eq!(
        registered_ids()
            .iter()
            .filter(|id| **id == "linked-registry-test")
            .count(),
        1
    );
    assert!(factory_for("linked-registry-test").is_some());
    assert!(factory_for("linked-registry-absent").is_none());
}
