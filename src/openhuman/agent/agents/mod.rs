mod loader;

// Built-in agents. Each module owns an `agent.toml` (metadata), the
// legacy `prompt.md` (kept alongside for reference / workspace
// overrides), and a `prompt.rs` exposing a `pub fn build(&PromptContext)
// -> Result<String>` that the loader wires into `PromptSource::Dynamic`.
pub mod archivist;
pub mod code_executor;
pub mod critic;
pub mod help;
pub mod integrations_agent;
pub mod morning_briefing;
pub mod orchestrator;
pub mod planner;
pub mod researcher;
pub mod summarizer;
pub mod tool_maker;
pub mod tools_agent;
pub mod trigger_reactor;
pub mod trigger_triage;
pub mod welcome;

pub use loader::{load_builtins, BuiltinAgent, BUILTINS};
