//! Test constructors for the copilot tool belt bound to this host.
//!
//! The tools are defined in `tinyflows_copilot::tools` and take ports; these
//! build them over [`FlowsHost`] for a given config, the way
//! [`registration`](super::super::registration) does, so the composition tests
//! exercise the real draft/flow stores and gate stack.

use std::sync::Arc;

use tinyflows_copilot::tools::{
    CancelFlowRunTool, CreateWorkflowTool, DryRunWorkflowTool, DuplicateFlowTool, EditWorkflowTool,
    GetFlowRunTool, GetFlowTool, GetNodeKindContractTool, ListFlowRunsTool, ListFlowsTool,
    ListNodeKindsTool, ResumeFlowRunTool, ReviseWorkflowTool, SaveWorkflowTool,
    ValidateWorkflowTool,
};

use crate::config::Config;
use crate::flows::builder_host::{FlowsHost, OverlayNodeKinds};

pub(crate) fn edit_workflow_tool(config: Arc<Config>) -> EditWorkflowTool {
    let h = FlowsHost::new(config);
    EditWorkflowTool::new(h.clone(), h.clone(), h)
}

pub(crate) fn save_workflow_tool(config: Arc<Config>) -> SaveWorkflowTool {
    let h = FlowsHost::new(config);
    SaveWorkflowTool::new(h.clone(), h.clone(), h)
}

pub(crate) fn dry_run_workflow_tool(config: Arc<Config>) -> DryRunWorkflowTool {
    let h = FlowsHost::new(config);
    DryRunWorkflowTool::new(h.clone(), h.clone(), h)
}

pub(crate) fn validate_workflow_tool(config: Arc<Config>) -> ValidateWorkflowTool {
    let h = FlowsHost::new(config);
    ValidateWorkflowTool::new(h.clone(), h.clone(), h)
}

pub(crate) fn revise_workflow_tool(config: Arc<Config>) -> ReviseWorkflowTool {
    ReviseWorkflowTool::new(FlowsHost::new(config))
}

pub(crate) fn create_workflow_tool(config: Arc<Config>) -> CreateWorkflowTool {
    let h = FlowsHost::new(config);
    CreateWorkflowTool::new(h.clone(), h)
}

pub(crate) fn duplicate_flow_tool(config: Arc<Config>) -> DuplicateFlowTool {
    DuplicateFlowTool::new(FlowsHost::new(config))
}

pub(crate) fn list_flow_runs_tool(config: Arc<Config>) -> ListFlowRunsTool {
    ListFlowRunsTool::new(FlowsHost::new(config))
}

pub(crate) fn cancel_flow_run_tool(config: Arc<Config>) -> CancelFlowRunTool {
    CancelFlowRunTool::new(FlowsHost::new(config))
}

pub(crate) fn resume_flow_run_tool(config: Arc<Config>) -> ResumeFlowRunTool {
    ResumeFlowRunTool::new(FlowsHost::new(config))
}

pub(crate) fn get_flow_tool(config: Arc<Config>) -> GetFlowTool {
    GetFlowTool::new(FlowsHost::new(config))
}

pub(crate) fn list_flows_tool(config: Arc<Config>) -> ListFlowsTool {
    ListFlowsTool::new(FlowsHost::new(config))
}

pub(crate) fn get_flow_run_tool(config: Arc<Config>) -> GetFlowRunTool {
    GetFlowRunTool::new(FlowsHost::new(config))
}

pub(crate) fn list_node_kinds_tool() -> ListNodeKindsTool {
    ListNodeKindsTool::new(Arc::new(OverlayNodeKinds))
}

pub(crate) fn get_node_kind_contract_tool() -> GetNodeKindContractTool {
    GetNodeKindContractTool::new(Arc::new(OverlayNodeKinds))
}
