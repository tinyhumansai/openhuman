use super::*;

#[tokio::test]
async fn memory_off_is_not_free() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = crate::memory::test_fixtures::config_in(&tmp);
    config.memory.engine = String::new();
    assert!(!free_period_active(&config).await);
}

#[tokio::test]
async fn an_engine_without_a_registration_is_always_free() {
    let tmp = tempfile::tempdir().unwrap();
    let config = crate::memory::test_fixtures::config_in(&tmp);
    crate::memory::test_fixtures::bind_reference(&config);
    assert!(free_period_active(&config).await);
}

#[tokio::test]
async fn the_hosted_engine_with_nothing_registered_is_not_free() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = crate::memory::test_fixtures::config_in(&tmp);
    config.memory.engine = engine::TINYHUMANS_ENGINE.to_string();
    assert!(!free_period_active(&config).await);
}
