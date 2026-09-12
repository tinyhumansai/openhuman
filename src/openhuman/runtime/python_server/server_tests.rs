use super::*;

#[tokio::test]
async fn prepare_launch_rejects_disabled_backends() {
    let mut config = Config::default();
    config.runtime_python.enabled = false;
    let err = prepare_launch(&config).await.unwrap_err().to_string();
    assert!(err.contains("no runtime python server backends enabled"));
}

#[test]
fn idle_timeout_expiry_uses_last_used_instant() {
    assert!(idle_timeout_expired(
        Instant::now() - Duration::from_secs(10),
        Duration::from_secs(5),
    ));
    assert!(!idle_timeout_expired(
        Instant::now(),
        Duration::from_secs(5),
    ));
}
