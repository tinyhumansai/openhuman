#[allow(clippy::module_inception)]
pub mod agent;
pub mod classifier;
pub mod cost;
pub mod dispatcher;
pub mod error;
pub mod events;
pub mod harness;
pub mod hooks;
pub mod host_runtime;
pub mod identity;
pub mod loop_;
pub mod memory_loader;
pub mod multimodal;
pub mod observer;
pub mod prompt;
mod schemas;
pub mod traits;
pub use schemas::{
    all_controller_schemas as all_agent_controller_schemas,
    all_registered_controllers as all_agent_registered_controllers,
};

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use agent::{Agent, AgentBuilder};
#[allow(unused_imports)]
pub use loop_::{process_message, run};
