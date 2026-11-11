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

fn native_rows() -> Vec<super::super::BalanceInfo> {
    vec![
        super::super::BalanceInfo {
            chain: tinywallet_bus::wire::WalletChain::Solana,
            evm_network: None,
            address: "Owner".into(),
            asset_symbol: "SOL".into(),
            decimals: 9,
            raw: "2500000000".into(),
            formatted: "2.500000000".into(),
            provider_status: super::super::ProviderStatus::Ready,
        },
        super::super::BalanceInfo {
            chain: tinywallet_bus::wire::WalletChain::Evm,
            evm_network: Some(tinywallet_bus::wire::EvmNetwork::BaseMainnet),
            address: "0xOwner".into(),
            asset_symbol: "ETH".into(),
            decimals: 18,
            raw: "0".into(),
            formatted: "0".into(),
            provider_status: super::super::ProviderStatus::Missing,
        },
    ]
}

fn tool_json(result: &ToolResult) -> Value {
    assert!(
        !result.is_error,
        "unexpected tool error: {}",
        result.output()
    );
    serde_json::from_str(&result.output()).expect("the tool answers with JSON")
}

#[test]
fn balances_tool_returns_native_and_token_rows_together() {
    let result = balances_tool_result(Ok(native_rows()), Ok(vec![token_row("1500000")])).unwrap();
    let value = tool_json(&result);
    assert_eq!(value["native"], json!(native_rows()));
    assert_eq!(value["tokens"][0]["assetSymbol"], "USDC");
    assert_eq!(value["tokens"][0]["raw"], "1500000");
    assert!(value.get("tokensError").is_none());
}

#[test]
fn a_failed_token_read_keeps_the_native_balances_and_says_why() {
    let result = balances_tool_result(Ok(native_rows()), Err("module too old".into())).unwrap();
    let value = tool_json(&result);
    assert_eq!(value["native"], json!(native_rows()));
    assert_eq!(value["tokens"], json!([]));
    assert_eq!(value["tokensError"], "module too old");
}

#[test]
fn a_failed_native_read_is_a_tool_error_even_when_tokens_were_read() {
    let result =
        balances_tool_result(Err("not configured".into()), Ok(vec![token_row("1")])).unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("not configured"));
}
