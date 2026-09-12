use super::*;

#[tokio::test]
async fn keyring_status_returns_ok() {
    let result = keyring_status().await;
    assert!(result.is_ok());
    let outcome = result.unwrap();
    assert!(!outcome.value.backend_name.is_empty());
}

#[tokio::test]
async fn keyring_consent_decide_rejects_invalid_mode() {
    let result = keyring_consent_decide("invalid".to_string()).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("invalid mode"));
}

#[tokio::test]
async fn keyring_retry_probe_returns_ok() {
    let result = keyring_retry_probe().await;
    assert!(result.is_ok());
}
