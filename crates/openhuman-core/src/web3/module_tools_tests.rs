use super::*;

fn tool(name: &str) -> Web3ModuleTool {
    Web3ModuleTool::new(
        tinywallet_bus::tools::find_web3_tool_declaration(name)
            .expect("a shipped web3 declaration"),
    )
}

#[test]
fn web3_execute_tools_are_external_effects_that_need_write_permission() {
    for name in [
        "web3_swap_execute",
        "web3_bridge_execute",
        "web3_dapp_execute",
    ] {
        let tool = tool(name);
        assert!(
            tool.external_effect_with_args(&serde_json::json!({"quoteId": "q", "confirmed": true})),
            "{name} broadcasts a transaction and must reach the approval gate"
        );
        assert_eq!(
            tool.permission_level_with_args(&serde_json::json!({})),
            PermissionLevel::Write,
            "{name}"
        );
    }
}

#[test]
fn web3_quote_and_route_tools_stay_read_only() {
    for name in [
        "web3_swap_quote",
        "web3_swap_routes",
        "web3_bridge_quote",
        "web3_dapp_call",
    ] {
        let tool = tool(name);
        assert!(
            !tool.external_effect_with_args(&serde_json::json!({})),
            "{name}"
        );
        assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly, "{name}");
    }
}
