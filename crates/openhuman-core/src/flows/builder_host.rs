//! The host half of the workflow-builder tool belt.
//!
//! The tools themselves (`edit_workflow`, `dry_run_workflow`, `save_workflow`,
//! ...) live in `tinyflows_copilot::tools` and run over ports. [`FlowsHost`] is
//! the one implementation of those ports for this process: it binds them to the
//! workspace's draft and flow stores, the config-aware gate stack, the agent
//! registry and the suggestion table, and adds no behaviour of its own.
//! [`OverlayNodeKinds`] is the node-kind catalog with this host's caveats
//! appended.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinyflows::caps::Capabilities;
use tinyflows::catalog::NodeKindContract;
use tinyflows::model::WorkflowGraph;
use tinyflows_catalog::types::FlowSuggestion;
use tinyflows_catalog::{DraftOrigin, Flow, FlowDraft, FlowRevision, FlowRun, FlowValidation};
use tinyflows_copilot::tools::ports::{
    AgentDefinition, AgentDirectory, BuilderGates, DraftStore, FlowStore, NodeKindSource,
    PortResult, SuggestionStore, Written,
};

use crate::config::Config;
use crate::flows::{ops, store};

/// The workspace-bound implementation of every copilot tool port.
pub struct FlowsHost {
    config: Arc<Config>,
}

impl FlowsHost {
    /// Binds the ports to `config`'s workspace.
    pub fn new(config: Arc<Config>) -> Arc<Self> {
        Arc::new(Self { config })
    }

    /// The context a port call runs under. The copilot ports carry none, so
    /// this is the one place the host reads the calling task's ambient scope
    /// (#7801) until the ports take a carrier.
    fn ctx(&self) -> crate::core::runtime::DispatchCtx {
        crate::core::runtime::DispatchCtx::ambient()
    }
}

#[async_trait]
impl DraftStore for FlowsHost {
    async fn get(&self, id: &str) -> PortResult<FlowDraft> {
        ops::flows_draft_get(&self.config, id).map(|o| o.value)
    }

    async fn create(
        &self,
        flow_id: Option<String>,
        name: String,
        graph: Value,
        origin: DraftOrigin,
    ) -> PortResult<FlowDraft> {
        ops::flows_draft_create(&self.config, flow_id, name, graph, origin).map(|o| o.value)
    }

    async fn update(
        &self,
        id: &str,
        name: Option<String>,
        graph: Option<Value>,
    ) -> PortResult<FlowDraft> {
        ops::flows_draft_update(&self.config, id, name, graph, None).map(|o| o.value)
    }
}

#[async_trait]
impl FlowStore for FlowsHost {
    async fn get(&self, id: &str) -> PortResult<Flow> {
        ops::flows_get(&self.config, id).await.map(|o| o.value)
    }

    async fn list(&self) -> PortResult<Vec<Flow>> {
        ops::flows_list(&self.config).await.map(|o| o.value)
    }

    async fn load_graph(&self, id: &str) -> PortResult<Option<WorkflowGraph>> {
        ops::load_flow_graph(&self.config, id)
    }

    async fn create(&self, name: String, graph: Value, require_approval: bool) -> PortResult<Flow> {
        ops::flows_create(&self.config, name, graph, require_approval)
            .await
            .map(|o| o.value)
    }

    async fn update(
        &self,
        id: &str,
        name: Option<String>,
        graph: Option<Value>,
    ) -> PortResult<Written<Flow>> {
        ops::flows_update(&self.config, id, name, graph, None, None)
            .await
            .map(|o| Written {
                value: o.value,
                logs: o.logs,
            })
    }

    async fn set_enabled(&self, id: &str, enabled: bool) -> PortResult<Flow> {
        ops::flows_set_enabled(&self.config, id, enabled)
            .await
            .map(|o| o.value)
    }

    async fn duplicate(&self, id: &str) -> PortResult<Flow> {
        ops::flows_duplicate(&self.config, id)
            .await
            .map(|o| o.value)
    }

    async fn history(&self, id: &str, limit: usize) -> PortResult<Vec<FlowRevision>> {
        ops::flows_get_history(&self.config, id, limit).map(|o| o.value)
    }

    async fn list_runs(&self, flow_id: &str, limit: usize) -> PortResult<Vec<FlowRun>> {
        ops::flows_list_runs(&self.config, flow_id, limit)
            .await
            .map(|o| o.value)
    }

    async fn get_run(&self, run_id: &str) -> PortResult<FlowRun> {
        ops::flows_get_run(&self.config, run_id)
            .await
            .map(|o| o.value)
    }

    async fn resume(
        &self,
        flow_id: &str,
        run_id: &str,
        approve: Vec<String>,
        reject: Vec<String>,
    ) -> PortResult<Value> {
        let ctx = self.ctx();
        ops::flows_resume(&ctx, &self.config, flow_id, run_id, approve, reject)
            .await
            .map(|o| o.value)
    }

    async fn cancel_run(&self, run_id: &str) -> PortResult<Value> {
        ops::flows_cancel_run(&self.config, run_id)
            .await
            .map(|o| o.value)
    }
}

#[async_trait]
impl BuilderGates for FlowsHost {
    async fn validate(&self, graph: Value) -> FlowValidation {
        ops::flows_validate(graph).value
    }

    async fn run_gates(&self, graph: &WorkflowGraph) -> Vec<String> {
        ops::run_builder_gates(&self.ctx(), &self.config, graph).await
    }

    async fn strict_gate(&self, graph: &Value) -> PortResult<()> {
        ops::strict_gate(&self.ctx(), &self.config, graph).await
    }

    async fn engine_compatibility_errors(&self, graph: &WorkflowGraph) -> Vec<String> {
        ops::config_aware_engine_compatibility_errors(&self.config, graph)
    }

    async fn wiring_warnings(&self, graph: &WorkflowGraph) -> Vec<String> {
        ops::graph_wiring_warnings(&self.config, graph).await
    }

    async fn approval_manifest(&self, graph: &WorkflowGraph) -> Vec<Value> {
        ops::compute_approval_manifest(&self.config, graph).await
    }

    async fn build_proposal(
        &self,
        retry_tool: &str,
        name: &str,
        graph: &WorkflowGraph,
        require_approval: bool,
        revision: bool,
        instruction: Option<String>,
        draft_id: Option<String>,
        flow_id: Option<String>,
    ) -> Result<Value, String> {
        ops::build_builder_proposal(
            &self.ctx(),
            &self.config,
            retry_tool,
            name,
            graph,
            require_approval,
            revision,
            instruction,
            draft_id,
            flow_id,
        )
        .await
    }

    fn sandbox_capabilities(&self) -> Capabilities {
        use crate::flows::tinyflows::caps::{
            PreflightToolInvoker, SchemaAwareMockAgentRunner, SchemaAwareMockLlm,
        };
        // The schema-aware mock `AgentRunner` makes a draft with `agent` nodes
        // exercise the agent-node path instead of erroring on a missing
        // capability, and honors `config.output_parser.schema` so the
        // null-resolution check does not false-positive on an agent node that
        // declared a schema. Plain agent nodes (no `agent_ref`) never reach the
        // runner; they go to the `llm` slot, so that is swapped for the
        // schema-aware mock too. The tool invoker is wrapped in the wiring
        // preflight, so a Composio `tool_call` whose required arg is missing or
        // null fails the dry run even over the echo mocks.
        let mut caps =
            tinyflows::caps::mock::mock_capabilities_with_agent(SchemaAwareMockAgentRunner);
        caps.llm = Arc::new(SchemaAwareMockLlm);
        caps.tools = Arc::new(PreflightToolInvoker {
            config: self.config.clone(),
            inner: caps.tools.clone(),
        });
        caps
    }
}

#[async_trait]
impl AgentDirectory for FlowsHost {
    async fn list(&self) -> PortResult<Vec<AgentDefinition>> {
        let agents = crate::agent::registry::list_agents(&self.ctx(), false)
            .await
            .map_err(|e| e.to_string())?;
        Ok(agents
            .iter()
            .map(|a| AgentDefinition {
                id: a.id.clone(),
                name: a.name.clone(),
                description: a.description.clone(),
                model: serde_json::to_value(&a.model).unwrap_or(Value::Null),
                tools: serde_json::to_value(&a.tool_allowlist).unwrap_or(Value::Null),
                tags: serde_json::to_value(&a.tags).unwrap_or(Value::Null),
            })
            .collect())
    }
}

#[async_trait]
impl SuggestionStore for FlowsHost {
    async fn upsert(&self, suggestions: &[FlowSuggestion]) -> PortResult<usize> {
        store::upsert_suggestions(&self.config, suggestions).map_err(|e| e.to_string())
    }
}

/// The portable node-kind catalog with this host's overlay applied
/// ([`node_contracts`](super::node_contracts)).
pub struct OverlayNodeKinds;

impl NodeKindSource for OverlayNodeKinds {
    fn all(&self) -> Vec<NodeKindContract> {
        super::node_contracts::all_node_kind_contracts()
    }

    fn get(&self, kind: &str) -> Option<NodeKindContract> {
        super::node_contracts::node_kind_contract(kind)
    }
}
