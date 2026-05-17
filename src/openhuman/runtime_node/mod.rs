//! Managed Node.js runtime and tool bridge.
//!
//! Responsibilities are split across submodules:
//!
//! * [`resolver`] — detect a compatible system `node` on `PATH`. Cheap,
//!   synchronous, called first so we can skip the download path when a
//!   matching toolchain already exists on the host.
//! * [`bootstrap`] / [`downloader`] / [`extractor`] — resolve or install the
//!   managed Node.js toolchain shipped with the core.
//! * [`ops`] / [`schemas`] — expose a minimal top-level runtime surface for
//!   listing agent-callable tools and dispatching a tool by name.

pub mod bootstrap;
pub mod downloader;
pub mod extractor;
pub mod ops;
pub mod resolver;
pub mod rpc;
mod schemas;
pub mod types;

pub use bootstrap::{NodeBootstrap, NodeSource, ResolvedNode};
pub use downloader::{download_distribution, fetch_shasums, NodeDistribution};
pub use extractor::{atomic_install, extract_distribution};
pub use ops::{execute_tool, list_tools};
pub use resolver::{detect_system_node, parse_node_version, SystemNode};
pub use schemas::{
    all_controller_schemas as all_runtime_node_controller_schemas,
    all_registered_controllers as all_runtime_node_registered_controllers,
};
