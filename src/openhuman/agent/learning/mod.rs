//! Agent self-learning subsystem.
//!
//! Post-turn hooks that reflect on completed turns, extract user preferences,
//! track tool effectiveness, and store learnings in the Memory backend.
//!
//! # Phase 1 additions (#566)
//!
//! - [`candidate`] — `LearningCandidate`, `FacetClass`, `CueFamily`, `EvidenceRef`,
//!   and the thread-safe ring-buffer [`candidate::Buffer`] that collects evidence
//!   from producers (Phase 2) for consumption by the stability detector (Phase 3).
//!
//! # Phase 2 additions (#566)
//!
//! - [`extract`] — producer modules: `signature` (email identity parser),
//!   `heuristics` (length-ratio + edit-window + correction-repeat detectors),
//!   `summary_facets` (structured facets from the LLM summariser).
//!
//! # Phase 3 additions (#566)
//!
//! - [`cache`] — `FacetCache` wrapper over `user_profile_facets` table.
//! - [`stability_detector`] — rebuild cycle: drain, aggregate, score, resolve, persist.
//! - [`scheduler`] — periodic + event-driven rebuild scheduling.

pub mod cache;
pub mod candidate;
pub mod extract;
pub mod linkedin_enrichment;
pub mod profile_md_renderer;
pub mod prompt_sections;
pub mod reflection;
pub mod scheduler;
pub mod schemas;
pub mod stability_detector;
pub mod startup;
/// In-memory profile fake for tests.
///
/// Not `#[cfg(test)]`: integration tests link the lib without it.
#[doc(hidden)]
pub mod test_profile;
pub mod tool_tracker;
pub mod tools;
pub mod transcript_ingest;
pub mod user_profile;

pub use cache::FacetCache;
pub use candidate::{Buffer, CueFamily, EvidenceRef, FacetClass, LearningCandidate};
pub use profile_md_renderer::ProfileMdRenderer;
pub use prompt_sections::{
    any_tool_offered, load_learned_from_cache, memory_write_instruction, LearnedContextSection,
    MemoryAccessSection, MemoryWriteSection, UserProfileSection, MEMORY_ACCESS_INSTRUCTION,
    MEMORY_READ_TOOLS, MEMORY_STORE_TOOL, MEMORY_WRITE_DELEGATE_TOOL, MEMORY_WRITE_TOOLS,
    SAVE_PREFERENCE_TOOL,
};
pub use reflection::ReflectionHook;
pub use schemas::{
    all_learning_controller_schemas, all_learning_registered_controllers, learning_schemas,
};
pub use stability_detector::StabilityDetector;
pub use tool_tracker::ToolTrackerHook;
pub use user_profile::UserProfileHook;
