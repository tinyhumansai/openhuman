use std::path::Path;

use super::types::SecurityPolicy;

#[tokio::test]
async fn missing_tinysecurity_refuses_protected_floor_and_path_authorization() {
    assert!(
        SecurityPolicy::is_always_forbidden_async(Path::new("/tmp/protected-test")).await,
        "an unavailable protected-path policy must fail closed"
    );

    let error = SecurityPolicy::default()
        .validate_path("/tmp/protected-test")
        .await
        .expect_err("file access needs TinySecurity authorization");
    assert!(
        error.contains("[policy-blocked] TinySecurity path authorization unavailable"),
        "{error}"
    );
}
