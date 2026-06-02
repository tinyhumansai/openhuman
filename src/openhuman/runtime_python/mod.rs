//! Managed Python runtime for Python-backed integrations.
//!
//! # Current consumers
//!
//! - **stdio MCP servers** implemented in Python (the original motivating
//!   use case).
//!
//! # Not currently consumed by `PresentationTool`
//!
//! The presentation tool used to run through `runtime_python` via a
//! `python-pptx` subprocess, but the #2778 engine refactor (commit
//! `e0e2a2e5`) swapped that out for the native-Rust `ppt-rs` crate
//! running in-process — see `src/openhuman/tools/impl/presentation/`.
//! The `run` + `venv` submodules below are intentionally retained as
//! scaffolding for the next Python-backed tool (e.g. a future
//! `pandas`/`numpy`-backed data-prep tool) so we don't have to
//! re-introduce interpreter discovery + venv provisioning from scratch.
//! Keep this module reachable from the workspace until the next consumer
//! lands; if no consumer materialises, delete the unused submodules as a
//! standalone cleanup PR rather than letting it bit-rot in tree.
//!
//! This module owns interpreter discovery and process-launch primitives
//! so callers do not need to care whether Python came from the host or a
//! future managed distribution.

pub mod bootstrap;
pub mod downloader;
pub mod extractor;
pub mod process;
pub mod resolver;
pub mod run;
pub mod venv;

pub use bootstrap::{PythonBootstrap, PythonSource, ResolvedPython};
pub use downloader::{fetch_release_metadata, select_distribution, PythonDistribution};
pub use extractor::{atomic_install, extract_distribution};
pub use process::PythonLaunchSpec;
pub use resolver::{detect_system_python, parse_python_version, PythonVersion, SystemPython};
pub use run::{run_python_script_to_completion, PythonRunOutput, PythonRunTimeout};
pub use venv::ensure_venv;
