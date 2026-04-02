//! Screen capture, accessibility automation, and vision summaries (macOS-focused).

pub mod ops;
mod schemas;

mod capture;
mod engine;
mod helpers;
mod image_processing;
mod limits;
mod permissions;
mod types;

pub use engine::{global_engine, AccessibilityEngine};
pub use ops as rpc;
pub use ops::*;
pub use schemas::{
    all_controller_schemas as all_screen_intelligence_controller_schemas,
    all_registered_controllers as all_screen_intelligence_registered_controllers,
};
pub use types::*;

#[cfg(test)]
mod tests;
