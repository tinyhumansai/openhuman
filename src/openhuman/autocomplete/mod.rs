//! Inline autocomplete domain — facade for the `desktop-automation` gate (#5049).
//!
//! The real engine/history/ops/schemas are `#[cfg(feature = "desktop-automation")]`;
//! the inert `types` module stays compiled in both directions (carve-out), and
//! `stub` re-exposes the always-on caller surface (`all_autocomplete_*`,
//! `global_engine`, `start_if_enabled`) when the feature is off.

// Inert request/response + status types (dep-free serde). Kept ungated and at
// the domain root so `autocomplete::AutocompleteStatus` (consumed by the
// always-compiled `app_state`) stays available when `desktop-automation` is
// off — the type carve-out from AGENTS.md.
pub mod types;

// Re-export the carved types at the domain root (ungated) so callers keep the
// historical `autocomplete::{AutocompleteStatus, …}` paths in both builds. This
// is the single re-export of these names; `core` reaches `super::types` directly.
pub use types::{
    AutocompleteAcceptParams, AutocompleteAcceptResult, AutocompleteCurrentParams,
    AutocompleteCurrentResult, AutocompleteDebugFocusResult, AutocompleteSetStyleParams,
    AutocompleteSetStyleResult, AutocompleteStartParams, AutocompleteStartResult,
    AutocompleteStatus, AutocompleteStopParams, AutocompleteStopResult, AutocompleteSuggestion,
};

#[cfg(feature = "desktop-automation")]
mod core;
#[cfg(feature = "desktop-automation")]
pub mod history;
#[cfg(feature = "desktop-automation")]
pub mod ops;
#[cfg(feature = "desktop-automation")]
mod schemas;

#[cfg(not(feature = "desktop-automation"))]
mod stub;
#[cfg(not(feature = "desktop-automation"))]
pub use stub::*;

#[cfg(feature = "desktop-automation")]
pub use core::*;
#[cfg(feature = "desktop-automation")]
pub use history::{
    clear_history, list_history, load_recent_examples, query_relevant_examples,
    save_accepted_completion, save_completion_to_local_docs, AcceptedCompletion,
};
#[cfg(feature = "desktop-automation")]
pub use ops as rpc;
#[cfg(feature = "desktop-automation")]
pub use ops::*;
#[cfg(feature = "desktop-automation")]
pub use schemas::{
    all_controller_schemas as all_autocomplete_controller_schemas,
    all_registered_controllers as all_autocomplete_registered_controllers,
};
