//! Agent loop: tool-call execution, CLI session, and channel message handling.

mod credentials;
mod history;
mod instructions;
mod memory_context;
mod parse;
mod session;
mod tool_loop;

pub(crate) use instructions::build_tool_instructions;
pub(crate) use parse::parse_tool_calls;
pub use session::{process_message, run};
pub(crate) use tool_loop::run_tool_call_loop;

#[cfg(test)]
mod tests;
