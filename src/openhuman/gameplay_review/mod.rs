//! Gameplay session review, highlight detection, and clip drafting.

pub mod ops;
mod schemas;
mod store;
pub mod types;

pub use ops as rpc;
pub use ops::*;
pub use schemas::{
    all_controller_schemas as all_gameplay_review_controller_schemas,
    all_registered_controllers as all_gameplay_review_registered_controllers,
};
pub use types::*;
