//! Text input intelligence — read, insert, and preview text in the OS-focused
//! input field.
//!
//! Thin orchestration layer consumed by autocomplete, voice control, and other
//! text-aware features. All platform work delegates to `accessibility::*`.

pub mod ops;
mod schemas;
mod types;

pub use ops as rpc;
pub use ops::*;
pub use schemas::{
    all_controller_schemas as all_text_input_controller_schemas,
    all_registered_controllers as all_text_input_registered_controllers,
};
pub use types::*;
