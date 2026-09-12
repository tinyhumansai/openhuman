use super::*;
use async_trait::async_trait;
use serde_json::json;

struct DummyTool;

#[async_trait]
impl Tool for DummyTool {
    fn name(&self) -> &str {
        "dummy_tool"
    }

    fn description(&self) -> &str {
        "Dummy tool"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
    ) -> anyhow::Result<crate::openhuman::skills::types::ToolResult> {
        Ok(crate::openhuman::skills::types::ToolResult::success("ok"))
    }
}

struct PolicyTool {
    name: &'static str,
    permission: PermissionLevel,
    external_effect: bool,
}

#[async_trait]
impl Tool for PolicyTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "Policy test tool"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }

    fn permission_level(&self) -> PermissionLevel {
        self.permission
    }

    fn external_effect_with_args(&self, _args: &serde_json::Value) -> bool {
        self.external_effect
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
    ) -> anyhow::Result<crate::openhuman::skills::types::ToolResult> {
        Ok(crate::openhuman::skills::types::ToolResult::success("ok"))
    }
}

#[test]
fn summarize_tool_exposes_metadata() {
    let summary = summarize_tool(&DummyTool);
    assert_eq!(summary.name, "dummy_tool");
    assert_eq!(summary.category, "system");
    assert_eq!(summary.permission_level, "ReadOnly");
    assert_eq!(summary.scope, "all");
}

#[test]
fn tool_scope_labels_are_stable() {
    assert_eq!(tool_scope_label(ToolScope::All), "all");
    assert_eq!(tool_scope_label(ToolScope::AgentOnly), "agent_only");
    assert_eq!(tool_scope_label(ToolScope::CliRpcOnly), "cli_rpc_only");
}

#[test]
fn command_class_for_tool_maps_metadata_to_policy_buckets() {
    let security = SecurityPolicy::default();

    let read = PolicyTool {
        name: "read_tool",
        permission: PermissionLevel::ReadOnly,
        external_effect: false,
    };
    assert_eq!(
        command_class_for_tool(&security, &read, &json!({})),
        CommandClass::Read
    );

    let write = PolicyTool {
        name: "write_tool",
        permission: PermissionLevel::Write,
        external_effect: false,
    };
    assert_eq!(
        command_class_for_tool(&security, &write, &json!({})),
        CommandClass::Write
    );

    let outbound = PolicyTool {
        name: "outbound_tool",
        permission: PermissionLevel::Write,
        external_effect: true,
    };
    assert_eq!(
        command_class_for_tool(&security, &outbound, &json!({})),
        CommandClass::Network
    );

    let dangerous = PolicyTool {
        name: "dangerous_tool",
        permission: PermissionLevel::Dangerous,
        external_effect: true,
    };
    assert_eq!(
        command_class_for_tool(&security, &dangerous, &json!({})),
        CommandClass::Destructive
    );
}

#[test]
fn command_class_for_shell_uses_command_args() {
    let security = SecurityPolicy::default();
    let shell = PolicyTool {
        name: "shell",
        permission: PermissionLevel::Execute,
        external_effect: true,
    };

    assert_eq!(
        command_class_for_tool(&security, &shell, &json!({"command": "ls src"})),
        CommandClass::Read
    );
    assert_eq!(
        command_class_for_tool(&security, &shell, &json!({"command": "touch out.txt"})),
        CommandClass::Write
    );
    assert_eq!(
        command_class_for_tool(
            &security,
            &shell,
            &json!({"command": "curl https://example.com"})
        ),
        CommandClass::Network
    );
    assert_eq!(
        command_class_for_tool(
            &security,
            &shell,
            &json!({"command": "cat Cargo.toml", "category": "destructive"})
        ),
        CommandClass::Destructive
    );
}
