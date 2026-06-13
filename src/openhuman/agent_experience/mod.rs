//! Hermes-style procedural experience memory for agents.

pub mod capture;
pub mod ops;
pub mod prompt;
pub mod schemas;
pub mod store;
pub mod types;

pub use capture::AgentExperienceCaptureHook;
pub use prompt::{prepend_experience_block, render_experience_hits, AGENT_EXPERIENCE_HEADING};
pub use schemas::{
    all_controller_schemas as all_agent_experience_controller_schemas,
    all_registered_controllers as all_agent_experience_registered_controllers,
};
pub use store::{AgentExperienceStore, ExperienceQuery, AGENT_EXPERIENCE_NAMESPACE};
pub use types::{
    redact_text, stable_experience_id, AgentExperience, ExperienceHit, ExperienceOutcome,
    ExperienceSource,
};
