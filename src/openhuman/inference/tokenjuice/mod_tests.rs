use super::*;

#[test]
fn recovery_tool_aliases_remain_stable() {
    assert!(is_recovery_tool(RETRIEVE_TOOL_NAME));
    assert!(is_recovery_tool(LEGACY_RETRIEVE_TOOL_NAME));
    assert!(!is_recovery_tool("shell"));
}

#[tokio::test]
async fn disabled_compaction_is_an_exact_pass_through_without_loading_the_module() {
    let content = "exact tool output".to_string();
    let output = compact_output_with_policy(
        content.clone(),
        "shell",
        false,
        AgentTokenjuiceCompression::Full,
    )
    .await;
    assert_eq!(output, content);
}

#[tokio::test]
async fn off_profile_is_an_exact_pass_through_without_loading_the_module() {
    let content = "exact tool output".to_string();
    let output = compact_output_with_policy(
        content.clone(),
        "shell",
        true,
        AgentTokenjuiceCompression::Off,
    )
    .await;
    assert_eq!(output, content);
}
