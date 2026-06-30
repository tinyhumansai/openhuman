pub(super) fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}
