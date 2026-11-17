use super::*;
use tinybus::module::ModuleCallError;

#[tokio::test]
async fn a_build_without_the_loader_never_falls_back_to_an_implementation() {
    let result = client(Config::default())
        .call::<serde_json::Value>("tinyhosts", "Providers", ())
        .await;
    assert_eq!(result, Err(ModuleCallError::Unavailable));
}

#[tokio::test]
async fn confidential_calls_are_unavailable_without_the_loader() {
    let result = client(Config::default())
        .call_confidential::<serde_json::Value>("tinywallet", "SignTransaction", ("secret",))
        .await;
    assert_eq!(result, Err(ModuleCallError::Unavailable));
}

#[tokio::test]
async fn unknown_ids_stay_unavailable_without_naming_the_id() {
    let error = client(Config::default())
        .call::<serde_json::Value>("private-user-path", "Anything", ())
        .await
        .unwrap_err();
    assert_eq!(error, ModuleCallError::Unavailable);
    assert!(!error.to_string().contains("private-user-path"));
}
