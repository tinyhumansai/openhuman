static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(super) fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}
