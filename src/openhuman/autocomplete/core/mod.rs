//! Autocomplete engine: macOS AX capture, local inline completion, overlay UI.

mod engine;
mod focus;
mod overlay;
mod terminal;
mod text;
mod types;

pub use engine::{global_engine, start_if_enabled, AutocompleteEngine, AUTOCOMPLETE_ENGINE};
pub use types::{
    AutocompleteAcceptParams, AutocompleteAcceptResult, AutocompleteCurrentParams,
    AutocompleteCurrentResult, AutocompleteDebugFocusResult, AutocompleteSetStyleParams,
    AutocompleteSetStyleResult, AutocompleteStartParams, AutocompleteStartResult,
    AutocompleteStatus, AutocompleteStopParams, AutocompleteStopResult, AutocompleteSuggestion,
};
