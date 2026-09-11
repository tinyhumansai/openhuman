//! Managed Python runtime for Python-backed integrations.
//!
//! [`bootstrap`] is the client for the `tinyruntime` module: it asks for an
//! interpreter and adapts the answer. Discovery, selection, download, and
//! install all live in the module now.
//!
//! [`process`] stays here because it is not runtime management. It launches the
//! long-lived stdio children this core owns — the runtime Python server, and the
//! stdio MCP servers — which outlive a single job and speak their own protocols.
//! The module resolves the interpreter; this core decides what to run with it.

pub mod bootstrap;
pub mod process;

pub use bootstrap::{PythonBootstrap, PythonSource, ResolvedPython};
pub use process::PythonLaunchSpec;
