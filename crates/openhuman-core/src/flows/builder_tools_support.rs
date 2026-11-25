//! Test constructors for the Copilot tools served by the compiled module.

use std::sync::Arc;

use crate::config::Config;
use crate::flows::copilot_bus::BusCopilotTool;

fn tool(config: Arc<Config>, name: &str) -> BusCopilotTool {
    BusCopilotTool::from_name(config, name)
}

pub(crate) fn propose_workflow_tool(config: Arc<Config>) -> BusCopilotTool {
    tool(config, "propose_workflow")
}

pub(crate) fn edit_workflow_tool(config: Arc<Config>) -> BusCopilotTool {
    tool(config, "edit_workflow")
}

pub(crate) fn save_workflow_tool(config: Arc<Config>) -> BusCopilotTool {
    tool(config, "save_workflow")
}

pub(crate) fn dry_run_workflow_tool(config: Arc<Config>) -> BusCopilotTool {
    tool(config, "dry_run_workflow")
}

pub(crate) fn validate_workflow_tool(config: Arc<Config>) -> BusCopilotTool {
    tool(config, "validate_workflow")
}

pub(crate) fn revise_workflow_tool(config: Arc<Config>) -> BusCopilotTool {
    tool(config, "revise_workflow")
}

pub(crate) fn create_workflow_tool(config: Arc<Config>) -> BusCopilotTool {
    tool(config, "create_workflow")
}

pub(crate) fn duplicate_flow_tool(config: Arc<Config>) -> BusCopilotTool {
    tool(config, "duplicate_flow")
}

pub(crate) fn list_flow_runs_tool(config: Arc<Config>) -> BusCopilotTool {
    tool(config, "list_flow_runs")
}

pub(crate) fn cancel_flow_run_tool(config: Arc<Config>) -> BusCopilotTool {
    tool(config, "cancel_flow_run")
}

pub(crate) fn resume_flow_run_tool(config: Arc<Config>) -> BusCopilotTool {
    tool(config, "resume_flow_run")
}

pub(crate) fn get_flow_tool(config: Arc<Config>) -> BusCopilotTool {
    tool(config, "get_flow")
}

pub(crate) fn list_flows_tool(config: Arc<Config>) -> BusCopilotTool {
    tool(config, "list_flows")
}

pub(crate) fn get_flow_run_tool(config: Arc<Config>) -> BusCopilotTool {
    tool(config, "get_flow_run")
}

pub(crate) fn list_node_kinds_tool(config: Arc<Config>) -> BusCopilotTool {
    tool(config, "list_node_kinds")
}

pub(crate) fn get_node_kind_contract_tool(config: Arc<Config>) -> BusCopilotTool {
    tool(config, "get_node_kind_contract")
}
