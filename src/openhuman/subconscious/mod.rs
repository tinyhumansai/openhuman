pub mod agent;
pub mod factory;
pub mod heartbeat;
pub mod instance;
pub mod profile;
pub mod profiles;
pub mod provider;
pub mod registry;
mod schemas;
pub mod session;
pub mod source_chunk;
pub mod store;
pub mod types;
pub mod user_thread;

pub use factory::{make_subconscious, SubconsciousKind};
pub use instance::SubconsciousInstance;
pub use profile::{Observation, Reflection, SubconsciousProfile};
pub use profiles::memory::memory_instance;

/// Back-compat alias for the old single-engine type. The live `memory` world is
/// now a [`SubconsciousInstance`] built via [`memory_instance`]; callers that
/// held a `SubconsciousEngine` keep the same runtime type.
pub type SubconsciousEngine = SubconsciousInstance;
pub use schemas::{
    all_controller_schemas as all_subconscious_controller_schemas,
    all_registered_controllers as all_subconscious_registered_controllers,
};
pub use session::{LongLivedSession, ProcessOutcome, ORCHESTRATOR_THREAD_ID};
pub use source_chunk::SourceChunk;
pub use types::{SubconsciousStatus, TickResult};
pub use user_thread::{notify_user, NotifyUserTool, USER_THREAD_ID};
