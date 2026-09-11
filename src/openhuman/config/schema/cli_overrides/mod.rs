//! Process-local inference overrides supplied by the standalone CLI.

mod ops;

pub use ops::AppliedInferenceOverride;
pub(crate) use ops::{
    apply_cli_inference_overrides, restore_persisted_inference_fields, set_cli_inference_overrides,
};
