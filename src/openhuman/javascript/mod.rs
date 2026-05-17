//! First-class JavaScript runtime surface.
//!
//! Today the implementation backend is the managed Node.js runtime in
//! [`crate::openhuman::runtime_node`]. This module exists so the rest of the
//! core talks to a language slot (`javascript`) rather than directly to a
//! specific backend. That keeps the door open for future sibling modules like
//! `python`, `ruby`, or a different JavaScript backend.

pub use crate::openhuman::runtime_node::types::{ExecuteToolOutcome, RuntimeToolSummary};
pub use crate::openhuman::runtime_node::{
    all_runtime_node_controller_schemas as all_javascript_controller_schemas,
    all_runtime_node_registered_controllers as all_javascript_registered_controllers,
};
pub use crate::openhuman::runtime_node::{
    atomic_install, detect_system_node, download_distribution, execute_tool, extract_distribution,
    fetch_shasums, list_tools, parse_node_version, NodeBootstrap, NodeDistribution, NodeSource,
    ResolvedNode, SystemNode,
};
