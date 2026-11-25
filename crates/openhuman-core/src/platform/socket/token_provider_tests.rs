use super::*;

#[tokio::test]
async fn static_provider_returns_token() {
    let provider = static_token_provider("my-token".to_string());
    assert_eq!(provider().await.unwrap(), "my-token");
}

#[tokio::test]
async fn static_provider_rejects_empty_token() {
    assert!(static_token_provider(String::new())().await.is_err());
    assert!(static_token_provider("   ".to_string())().await.is_err());
}

#[tokio::test]
async fn static_provider_returns_same_token_on_repeated_calls() {
    let provider = static_token_provider("tok-abc".to_string());
    assert_eq!(provider().await.unwrap(), provider().await.unwrap());
}
