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

fn token_row(raw: &str) -> super::super::TokenBalanceInfo {
    super::super::TokenBalanceInfo {
        chain: tinywallet_bus::wire::WalletChain::Solana,
        evm_network: None,
        address: "Owner".into(),
        asset_symbol: "USDC".into(),
        token_address: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into(),
        decimals: 6,
        raw: raw.into(),
        formatted: "1.500000".into(),
        provider_status: super::super::ProviderStatus::Ready,
    }
}

#[test]
fn balances_tool_returns_native_and_token_rows_together() {
    let value = combine_balances(Ok(Vec::new()), Ok(vec![token_row("1500000")])).unwrap();
    assert_eq!(value["native"], json!([]));
    assert_eq!(value["tokens"][0]["assetSymbol"], "USDC");
    assert_eq!(value["tokens"][0]["raw"], "1500000");
    assert!(value.get("tokensError").is_none());
}

#[test]
fn a_failed_token_read_keeps_the_native_balances_and_says_why() {
    let value = combine_balances(Ok(Vec::new()), Err("module too old".into())).unwrap();
    assert_eq!(value["tokens"], json!([]));
    assert_eq!(value["tokensError"], "module too old");
    assert!(combine_balances(Err("not configured".into()), Ok(Vec::new())).is_err());
}
