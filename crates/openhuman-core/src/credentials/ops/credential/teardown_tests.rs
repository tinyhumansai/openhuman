use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[tokio::test]
async fn device_pairing_lock_finishes_before_credential_scope_teardown() {
    let locked = Arc::new(AtomicBool::new(false));
    let scope_deactivated = Arc::new(AtomicBool::new(false));
    let lock_result = Arc::clone(&locked);
    let teardown_lock = Arc::clone(&locked);
    let teardown_deactivated = Arc::clone(&scope_deactivated);

    let result = after_device_pairing_lock(
        || async move {
            lock_result.store(true, Ordering::SeqCst);
        },
        || async move {
            assert!(teardown_lock.load(Ordering::SeqCst));
            teardown_deactivated.store(true, Ordering::SeqCst);
            Ok::<(), String>(())
        },
    )
    .await;

    assert!(result.is_ok());
    assert!(locked.load(Ordering::SeqCst));
    assert!(scope_deactivated.load(Ordering::SeqCst));
}
