//! First-class JavaScript runtime surface.
//!
//! The implementation backend is the managed Node.js toolchain in
//! [`crate::openhuman::runtime::node`], which is itself a client for the
//! `tinyruntime` module. This facade exists so the rest of the core talks to a
//! language slot (`javascript`) rather than to a specific backend — which is
//! also how the module underneath was swapped without the callers noticing.
//!
//! ## Gating (`runtime-node`)
//!
//! The facade itself is always compiled — `ShellTool` imports [`NodeBootstrap`]
//! through it — but the re-exports split. The bootstrap type surface comes from
//! `node`'s stub when the feature is off; the dispatch machinery and the
//! controller pair are gated, because their only consumers are gated off too.

pub use crate::openhuman::runtime::node::{
    ExecuteToolOutcome, NodeBootstrap, NodeSource, ResolvedNode,
};

#[cfg(feature = "runtime-node")]
pub use crate::openhuman::runtime::node::types::RuntimeToolSummary;
#[cfg(feature = "runtime-node")]
pub use crate::openhuman::runtime::node::{
    all_runtime_node_controller_schemas as all_javascript_controller_schemas,
    all_runtime_node_registered_controllers as all_javascript_registered_controllers,
};
#[cfg(feature = "runtime-node")]
pub use crate::openhuman::runtime::node::{execute_tool, list_tools};
