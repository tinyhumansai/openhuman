use super::*;

#[test]
fn tool_metadata() {
    let tool = CompleteOnboardingTool::new();
    assert_eq!(tool.name(), "complete_onboarding");
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
    assert_eq!(tool.scope(), ToolScope::AgentOnly);
    let schema = tool.parameters_schema();
    assert_eq!(schema["type"], "object");
    // No required params — call it with `{}`.
    assert!(schema.get("required").is_none());
}

#[test]
fn description_mentions_check_onboarding_status() {
    let desc = CompleteOnboardingTool::new().description().to_string();
    assert!(
        desc.contains("check_onboarding_status"),
        "description should point agents at the companion status tool: {desc}"
    );
    assert!(desc.contains("ready_to_complete"));
}

#[test]
fn spec_roundtrip() {
    let tool = CompleteOnboardingTool::new();
    let spec = tool.spec();
    assert_eq!(spec.name, "complete_onboarding");
    assert!(spec.parameters.is_object());
}
