mod loader;

// Built-in agents. Each module owns an `agent.toml` (metadata), the
// legacy `prompt.md` (kept alongside for reference / workspace
// overrides), and a `prompt.rs` exposing a `pub fn build(&PromptContext)
// -> Result<String>` that the loader wires into `PromptSource::Dynamic`.
pub mod account_admin_agent;
pub mod archivist;
pub mod code_executor;
pub mod context_scout;
pub mod critic;
pub mod crypto_agent;
pub mod desktop_control_agent;
pub mod goals_agent;
pub mod help;
pub mod integrations_agent;
pub mod markets_agent;
pub mod mcp_agent;
pub mod mcp_setup;
pub mod morning_briefing;
pub mod orchestrator;
pub mod planner;
pub mod presentation_agent;
pub mod profile_memory_agent;
pub mod researcher;
pub mod scheduler_agent;
pub mod screen_awareness_agent;
pub mod settings_agent;
pub mod skill_creator;
pub mod summarizer;
pub mod task_manager_agent;
pub mod tool_maker;
pub mod tools_agent;
pub mod trigger_reactor;
pub mod trigger_triage;
pub mod vision_agent;

pub use loader::{load_builtins, validate_tier_hierarchy, BuiltinAgent, BUILTINS};
