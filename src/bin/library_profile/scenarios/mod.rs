//! One module per profiling scenario. Each exposes a single
//! `run() -> Result<ProfileResult>` entry point dispatched from `main`.

pub mod agent_turn;
pub mod cold_phases;
pub mod fleet;
pub mod long_agent;
pub mod memory_ingest;
pub mod skill_run;
pub mod subagent_storm;
pub mod subagents;
pub mod subconscious;
pub mod workflow;
