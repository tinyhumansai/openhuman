use super::*;
use openhuman_core::agent::tool_policy::{ToolCallContext, ToolPolicyDecision};
struct Deny;
#[async_trait::async_trait]
impl ToolPolicy for Deny {
    fn name(&self) -> &str {
        "deny"
    }
    async fn check(&self, _: &ToolPolicyRequest) -> ToolPolicyDecision {
        ToolPolicyDecision::Deny {
            reason: "host denied".into(),
        }
    }
}
fn request(name: &str) -> ToolPolicyRequest {
    ToolPolicyRequest::new(
        name,
        serde_json::json!({}),
        ToolCallContext::session("s", "c", "a", "call", 0),
    )
}
#[tokio::test]
async fn attachment_policy_does_not_replace_original_host_authority() {
    let policy = AttachmentPolicy {
        original: Some(Arc::new(Deny)),
        policies: vec![
            (HashSet::from(["hivemind_message".into()]), None),
            (
                HashSet::from(["hivemind_create".into()]),
                Some(Arc::new(Deny)),
            ),
        ],
    };
    assert!(matches!(
        policy.check(&request("hivemind_message")).await,
        ToolPolicyDecision::Allow
    ));
    assert!(matches!(
        policy.check(&request("host_original")).await,
        ToolPolicyDecision::Deny { .. }
    ));
    assert!(matches!(
        policy.check(&request("hivemind_create")).await,
        ToolPolicyDecision::Deny { .. }
    ));
}
