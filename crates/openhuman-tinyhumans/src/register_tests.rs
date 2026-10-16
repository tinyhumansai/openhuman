use super::*;

#[test]
fn register_all_is_idempotent() {
    let _registry = crate::registry_test_lock();

    let options = InstallOptions::default();
    register_all(&options);
    register_all(&options);
}
