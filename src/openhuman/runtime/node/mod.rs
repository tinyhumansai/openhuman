//! Managed Node.js runtime and the generic tool bridge.
//!
//! Two unrelated things share this directory, and the difference matters:
//!
//! * [`bootstrap`] is the *client* for the `tinyruntime` module. It asks for a
//!   Node toolchain and adapts the answer. It downloads nothing, unpacks
//!   nothing, and manages no cache — the module owns all of that now.
//! * [`ops`] and [`types`] are the generic native-tool dispatcher over the agent
//!   tool registry (`oh:*` tools such as `memory_search`, file, and shell
//!   tools). They have nothing to do with Node beyond being reachable from
//!   JavaScript, which is why they are not gated below.
//!
//! ## Gating (`runtime-node`)
//!
//! The module is always declared, but the managed-Node client and the
//! `javascript.*` controller pair are `#[cfg(feature = "runtime-node")]`; a
//! [`stub`] carries [`NodeBootstrap`]'s type surface when the feature is off.
//! The forcing constraint is `ShellTool`, which holds
//! `Option<Arc<NodeBootstrap>>` as a field and is kernel — always compiled.
//!
//! [`ops`] and [`types`] stay ungated because they back both the gated
//! `javascript.*` controllers *and* the ungated `flows` `oh:` backend, which
//! must keep dispatching native tools when the managed Node runtime is compiled
//! out.

#[cfg(feature = "runtime-node")]
pub mod bootstrap;
#[cfg(feature = "runtime-node")]
pub mod rpc;
#[cfg(feature = "runtime-node")]
mod schemas;

/// Generic runtime tool bridge. Always compiled — see module docs.
pub mod ops;
/// Inert serde types shared by [`ops`] and the gated JS RPC. Always compiled.
pub mod types;

#[cfg(not(feature = "runtime-node"))]
mod stub;
#[cfg(not(feature = "runtime-node"))]
pub use stub::{NodeBootstrap, NodeSource, ResolvedNode, RUNTIME_NODE_DISABLED_MESSAGE};

#[cfg(feature = "runtime-node")]
pub use bootstrap::{NodeBootstrap, NodeSource, ResolvedNode};
pub use ops::{execute_tool, list_tools};
#[cfg(feature = "runtime-node")]
pub use schemas::{
    all_controller_schemas as all_runtime_node_controller_schemas,
    all_registered_controllers as all_runtime_node_registered_controllers,
};
pub use types::ExecuteToolOutcome;
