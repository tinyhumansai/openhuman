use super::*;
struct Deny;
#[async_trait::async_trait]
impl ToolCallPolicy<OpenHumanRunContext> for Deny {
    fn name(&self) -> &str {
        "deny"
    }
    async fn check(&self, _: &RunContext<OpenHumanRunContext>, _: &ToolCall) -> PolicyDecision {
        PolicyDecision::Deny {
            reason: "host denied".into(),
        }
    }
}
fn request(name: &str) -> (RunContext<OpenHumanRunContext>, ToolCall) {
    (
        RunContext::new(
            tinyagents_harness::context::RunConfig::new("attachments-test"),
            OpenHumanRunContext::new(),
        ),
        ToolCall::new("call", name, serde_json::json!({})),
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
        {
            let (ctx, call) = request("hivemind_message");
            policy.check(&ctx, &call).await
        },
        PolicyDecision::Allow
    ));
    assert!(matches!(
        {
            let (ctx, call) = request("host_original");
            policy.check(&ctx, &call).await
        },
        PolicyDecision::Deny { .. }
    ));
    assert!(matches!(
        {
            let (ctx, call) = request("hivemind_create");
            policy.check(&ctx, &call).await
        },
        PolicyDecision::Deny { .. }
    ));
}
