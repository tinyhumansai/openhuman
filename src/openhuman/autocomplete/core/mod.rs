//! Autocomplete engine: macOS AX capture, local inline completion, overlay UI.

mod engine;
mod focus;
mod overlay;
mod terminal;
mod text;

pub use engine::{global_engine, start_if_enabled, AutocompleteEngine, AUTOCOMPLETE_ENGINE};
// The inert request/response + status types live one level up in
// `autocomplete::types` (dep-free) and are re-exported by the `autocomplete`
// facade (ungated). `core`'s own code reaches them via `super::super::types`; the
// facade owns the public `autocomplete::{AutocompleteStatus, …}` re-export, so we
// do not re-export them here (that would collide with the facade's `pub use`).
