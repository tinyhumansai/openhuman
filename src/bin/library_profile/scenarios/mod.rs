//! One module per profiling scenario. Each exposes a single
//! `run() -> Result<ProfileResult>` entry point dispatched from `main`.
//!
//! `memory-ingest` and `cold-phases` are absent. Both measured the memory
//! engine embedded in this process — `memory_ingest` drained its queue,
//! `cold_phases` checkpointed its bootstrap through
//! `tinymemory_core::store::MemoryClient` — and this binary no longer links
//! one (openhuman#6161). Re-adding them means measuring the memory *module*
//! over the bus, which is a different scenario and wants a fresh design
//! rather than a revived file.

pub mod agent_turn;
pub mod fleet;
pub mod long_agent;
pub mod skill_run;
pub mod subagent_storm;
pub mod workflow;
