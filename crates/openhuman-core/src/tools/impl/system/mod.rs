//! Cross-cutting system tools: shell, Node/npm/Python execution, tool
//! detection/install, time helpers, LSP, proxy config.
//! Each family, its tool structs, and its registration gates are catalogued in
//! `tools/impl/README.md`.

mod exec_adapters;
mod install_tool;
mod lsp;
mod retrieve_tool_output;
mod schedule;
mod shell;
mod shell_description;
mod shell_process;
mod shell_run;
mod shell_saas;
mod shell_support;

pub use crate::config::proxy_tool::ProxyConfigTool;
pub use install_tool::InstallToolTool;
pub use lsp::{lsp_capability_enabled, LspTool, LSP_ENABLED_ENV};
pub use retrieve_tool_output::{retrieve_tool_output_tool, RetrieveToolOutputTool};
pub use schedule::ScheduleTool;
pub use shell::ShellTool;
pub use shell_process::ShellProcessTool;
pub(crate) use shell_support::YIELD_AFTER_ARG;
