//! Screen capture, accessibility automation, and vision summaries (macOS-focused).
//!
//! Facade for the `desktop-automation` gate (#5049): the capture/vision/engine/
//! server/cli/tools submodules are `#[cfg(feature = "desktop-automation")]`; the
//! inert `types` module (`AccessibilityStatus`, `CaptureImageRefResult`, the
//! session/permission structs) stays compiled in both directions (carve-out), and
//! `stub` re-exposes the always-on caller surface (`all_screen_intelligence_*`,
//! `global_engine`, `server::{start_if_enabled, try_global_server}`,
//! `rpc::accessibility_capture_image_ref`, `cli`) when the feature is off.

// Inert serde types — compiled in BOTH builds (type carve-out). Consumed by the
// always-on `app_state` (literal `AccessibilityStatus`) and `tools::local_cli`
// (`CaptureImageRefResult`). Re-exports the carved `accessibility` types.
mod types;

#[cfg(feature = "desktop-automation")]
pub(crate) mod cli;
#[cfg(feature = "desktop-automation")]
pub mod ops;
#[cfg(feature = "desktop-automation")]
mod schemas;
#[cfg(feature = "desktop-automation")]
pub mod server;
#[cfg(feature = "desktop-automation")]
pub mod tools;

#[cfg(feature = "desktop-automation")]
mod capture;
#[cfg(feature = "desktop-automation")]
mod capture_worker;
#[cfg(feature = "desktop-automation")]
mod engine;
#[cfg(feature = "desktop-automation")]
mod helpers;
#[cfg(feature = "desktop-automation")]
mod image_processing;
#[cfg(feature = "desktop-automation")]
mod input;
#[cfg(feature = "desktop-automation")]
mod limits;
#[cfg(feature = "desktop-automation")]
mod permissions;
#[cfg(feature = "desktop-automation")]
mod processing_worker;
#[cfg(feature = "desktop-automation")]
mod state;
#[cfg(feature = "desktop-automation")]
mod vision;

#[cfg(not(feature = "desktop-automation"))]
mod stub;
#[cfg(not(feature = "desktop-automation"))]
pub use stub::*;

#[cfg(feature = "desktop-automation")]
pub use ops as rpc;
#[cfg(feature = "desktop-automation")]
pub use ops::*;
#[cfg(feature = "desktop-automation")]
pub use schemas::{
    all_controller_schemas as all_screen_intelligence_controller_schemas,
    all_registered_controllers as all_screen_intelligence_registered_controllers,
};
#[cfg(feature = "desktop-automation")]
pub use state::{global_engine, AccessibilityEngine};

// Carved types — compiled in BOTH builds.
pub use types::*;

#[cfg(all(test, feature = "desktop-automation"))]
mod tests;
