//! Builder tools backed by the Composio catalog and connected accounts.
//!
//! | Tool | Purpose |
//! | --- | --- |
//! | `SearchToolCatalogTool` | Search the live catalog and rank curated actions first. |
//! | `GetToolContractTool` | Read the complete contract for one action. |
//! | `GetToolOutputSampleTool` | Observe one bounded read-only action result. |
//! | `ListConnectableToolkitsTool` | List supported toolkits and connection state. |
//! | `ListFlowConnectionsTool` | Read the host-provided non-secret flow connection list. |

pub(crate) mod catalog_search;
mod connection_reads;
mod tool_contract;

pub(crate) use catalog_search::search_live_catalog;
pub use catalog_search::SearchToolCatalogTool;
pub use connection_reads::{
    ConnectableToolkitProvider, FlowConnectionProvider, ListConnectableToolkitsTool,
    ListFlowConnectionsTool,
};
pub use tool_contract::{GetToolContractTool, GetToolOutputSampleTool};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
