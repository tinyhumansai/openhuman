use super::*;

#[test]
fn execute_tool_is_an_external_effect_outside_trusted_automation() {
    // No turn scope here: the gate must treat the call as needing approval.
    let tool = WalletExecutePreparedTool;
    assert!(tool.external_effect_with_args(&json!({"quoteId": "q", "confirmed": true})));
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
}

#[tokio::test]
async fn execute_tool_is_governed_by_policy_not_the_gate_in_a_background_turn() {
    use crate::core::runtime::turn_origin::{
        with_origin, AgentTurnOrigin, TrustedAutomationSource,
    };
    let tool = WalletExecutePreparedTool;
    let background = AgentTurnOrigin::TrustedAutomation {
        job_id: "job".into(),
        source: TrustedAutomationSource::Background,
    };
    let external = with_origin(background, async {
        tool.external_effect_with_args(&json!({"quoteId": "q", "confirmed": true}))
    })
    .await;
    assert!(!external);
    let reviewed = AgentTurnOrigin::TrustedAutomation {
        job_id: "flow".into(),
        source: TrustedAutomationSource::Workflow {
            require_approval: true,
        },
    };
    let external = with_origin(reviewed, async {
        tool.external_effect_with_args(&json!({"quoteId": "q", "confirmed": true}))
    })
    .await;
    assert!(
        external,
        "a workflow that requires approval still parks at the gate"
    );
}

#[tokio::test]
async fn execute_tool_refuses_an_external_channel_turn_before_touching_the_wallet() {
    use crate::core::runtime::turn_origin::{with_origin, AgentTurnOrigin};
    let channel = AgentTurnOrigin::ExternalChannel {
        channel: "telegram".into(),
        sender: Some("u".into()),
        sender_name: None,
        reply_target: "r".into(),
        message_id: "m".into(),
        history_key: None,
        channel_session: None,
    };
    let result = with_origin(channel, async {
        run_execute(
            &DispatchCtx::ambient_turn(),
            json!({"quoteId": "q", "confirmed": true}),
            None,
        )
        .await
    })
    .await;
    assert!(result.is_error);
    assert!(result.output().contains("WALLET_POLICY_REFUSED"));
}

#[test]
fn balance_tool_is_a_read() {
    let tool = WalletBalancesTool;
    assert!(!tool.external_effect_with_args(&json!({})));
    assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
    assert_eq!(automation_tools().len(), 2);
}

#[tokio::test]
async fn execute_tool_is_policy_governed_for_scheduled_actions_cron_but_not_plain_cron() {
    use crate::core::runtime::turn_origin::{
        with_origin, AgentTurnOrigin, TrustedAutomationSource,
    };
    let tool = WalletExecutePreparedTool;
    let args = json!({"quoteId": "q", "confirmed": true});
    for (source, external) in [
        (TrustedAutomationSource::CronActions, false),
        (TrustedAutomationSource::Cron, true),
    ] {
        let origin = AgentTurnOrigin::TrustedAutomation {
            job_id: "job".into(),
            source,
        };
        let got = with_origin(origin, async { tool.external_effect_with_args(&args) }).await;
        assert_eq!(got, external);
    }
}

#[tokio::test]
async fn execute_tool_refuses_a_plain_cron_turn_before_touching_the_wallet() {
    use crate::core::runtime::turn_origin::{
        with_origin, AgentTurnOrigin, TrustedAutomationSource,
    };
    let cron = AgentTurnOrigin::TrustedAutomation {
        job_id: "job".into(),
        source: TrustedAutomationSource::Cron,
    };
    let result = with_origin(cron, async {
        run_execute(
            &DispatchCtx::ambient_turn(),
            json!({"quoteId": "q", "confirmed": true}),
            None,
        )
        .await
    })
    .await;
    assert!(result.is_error);
    assert!(result.output().contains("WALLET_POLICY_REFUSED"));
}
