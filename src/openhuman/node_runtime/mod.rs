//! Managed Node.js runtime for skills that require `node` / `npm`.
//!
//! Responsibilities are split across submodules:
//!
//! * [`resolver`] — detect a compatible system `node` on `PATH`. Cheap,
//!   synchronous, called first so we can skip the download path when a
//!   matching toolchain already exists on the host.
//!
//! Later commits layer on a downloader, archive extractor, cache manager,
//! and a bootstrap entry point that returns the resolved `node`/`npm`
//! binary paths for `node_exec` / `npm_exec` tools.

pub mod downloader;
pub mod extractor;
pub mod resolver;

pub use downloader::{download_distribution, fetch_shasums, NodeDistribution};
pub use extractor::{atomic_install, extract_distribution};
pub use resolver::{detect_system_node, parse_node_version, SystemNode};
