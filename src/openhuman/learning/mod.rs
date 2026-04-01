//! Agent self-learning subsystem.
//!
//! Post-turn hooks that reflect on completed turns, extract user preferences,
//! track tool effectiveness, and store learnings in the Memory backend.

pub mod prompt_sections;
pub mod reflection;
pub mod tool_tracker;
pub mod user_profile;

pub use prompt_sections::{LearnedContextSection, UserProfileSection};
pub use reflection::ReflectionHook;
pub use tool_tracker::ToolTrackerHook;
pub use user_profile::UserProfileHook;
