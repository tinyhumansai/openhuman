//! The `frontend_agent` built-in: the Quick-LLM front end of the orchestration
//! wake graph. Registered in the built-in loader
//! ([`crate::openhuman::agent_registry::agents::loader`]).

pub mod graph;
pub mod prompt;
