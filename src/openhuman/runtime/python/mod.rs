//! Managed Python runtime for Python-backed integrations.
//!
//! The immediate use case is stdio MCP servers implemented in Python. This
//! module owns interpreter discovery and process-launch primitives so callers
//! do not need to care whether Python came from the host or a future managed
//! distribution.

pub mod bootstrap;
pub mod downloader;
pub mod extractor;
pub mod process;
pub mod resolver;

pub use bootstrap::{PythonBootstrap, PythonSource, ResolvedPython};
pub use downloader::{fetch_release_metadata, select_distribution, PythonDistribution};
pub use extractor::{atomic_install, extract_distribution};
pub use process::PythonLaunchSpec;
pub use resolver::{detect_system_python, parse_python_version, PythonVersion, SystemPython};
