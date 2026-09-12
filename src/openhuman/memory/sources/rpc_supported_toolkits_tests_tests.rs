use super::*;

/// The supported-toolkit set must include every built-in provider slug.
/// Asserted via `contains` (not exact equality) because the provider
/// registry is a process-global shared with other tests in this binary
/// that may register ad-hoc dummy providers.
#[tokio::test]
async fn supported_toolkits_includes_builtin_providers() {
    let outcome = supported_toolkits_rpc()
        .await
        .expect("supported_toolkits_rpc should succeed");
    let toolkits = outcome.value.toolkits;

    for slug in ["clickup", "github", "gmail", "linear", "notion", "slack"] {
        assert!(
            toolkits.iter().any(|t| t == slug),
            "expected supported toolkits to include '{slug}', got {toolkits:?}"
        );
    }
}

/// The returned set must be sorted and free of duplicates.
#[tokio::test]
async fn supported_toolkits_is_sorted_and_deduped() {
    let outcome = supported_toolkits_rpc()
        .await
        .expect("supported_toolkits_rpc should succeed");
    let toolkits = outcome.value.toolkits;

    let mut sorted = toolkits.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        toolkits, sorted,
        "toolkits should be sorted and de-duplicated"
    );
}
