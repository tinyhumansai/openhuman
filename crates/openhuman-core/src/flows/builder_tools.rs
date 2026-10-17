//! The Composio-bound half of the `workflow-builder` tool belt.
//!
//! The authoring tools (`revise_workflow`, `edit_workflow`, `validate_workflow`,
//! `dry_run_workflow`, `create_workflow`, `duplicate_flow`, `save_workflow`, the
//! run controls, the flow reads, `list_agent_definitions`, the node-kind reads
//! and `suggest_workflows`) live in `tinyflows_copilot::tools` and run over the
//! ports [`super::builder_host::FlowsHost`] implements; their registration is in
//! [`super::registration`]. This module keeps the tools that are bound to the
//! live Composio catalog and the connected accounts, which only this host owns:
//!
//! | Tool                             | Permission | Effect                                                        |
//! | --------------------------------- | ---------- | ---------------------------------------------------------------- |
//! | [`ListConnectableToolkitsTool`]   | `None`     | read: which toolkits are already connected                        |
//! | [`ListFlowConnectionsTool`]       | `None`     | read: connection refs (ids/names only)                            |
//! | [`SearchToolCatalogTool`]         | `None`     | read: real Composio tool slugs (live catalog)                     |
//! | [`GetToolContractTool`]           | `None`     | read: one action's FULL live contract                             |
//! | [`GetToolOutputSampleTool`]       | `ReadOnly` | ONE bounded real Composio call (Read-scope only, connected toolkit only) |
//!
//! The agent's full tool scope (see `agent_registry/agents/workflow_builder/
//! agent.toml`) also grants the Composio **discovery/connect** tools —
//! `composio_list_toolkits`, `composio_list_connections`, `composio_connect`
//! (defined in `composio/tools.rs`) — so the builder can link an app the
//! workflow needs before proposing. Those stay within the copilot's
//! human-in-the-loop invariant: connect is an approval-gated OAuth hand-off, and
//! `composio_execute` (running an arbitrary real action, any scope) remains
//! deliberately OUT of scope.
//!
//! **One narrow, deliberate carve-out (B12):** [`GetToolOutputSampleTool`]
//! (`get_tool_output_sample`) DOES perform a real Composio call — but only
//! ever a `Read`-scope one (hard-refused otherwise, regardless of the user's
//! per-toolkit scope preference), and only against a toolkit the user has
//! ALREADY connected. It exists because some actions' live listings publish
//! no output schema at all (verified for every GitHub action), leaving
//! `get_tool_contract` with no ground truth for a downstream `split_out.path`
//! — this makes exactly one bounded real read to observe the actual shape
//! instead. It can never send/create/update/delete anything.

mod catalog_search;
mod connection_reads;
mod tool_contract;

pub(crate) use catalog_search::search_live_catalog;
pub use catalog_search::SearchToolCatalogTool;
pub use connection_reads::{ListConnectableToolkitsTool, ListFlowConnectionsTool};
pub use tool_contract::{GetToolContractTool, GetToolOutputSampleTool};

#[cfg(test)]
use connection_reads::flow_connection_to_json;

#[cfg(test)]
#[path = "builder_tools_support.rs"]
pub(crate) mod tests_support;

#[cfg(test)]
#[path = "builder_tools_tests.rs"]
mod tests;
