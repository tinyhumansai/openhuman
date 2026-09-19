//! Announcements RPC adapter — thin-wraps the hosted API.
//!
//! Exposes the latest active announcement for the signed-in user through the
//! standard controller registry (`openhuman.announcements_*`). The UI surfaces
//! it once on harness init and tracks dismissal locally by id.

mod ops;
mod schemas;

pub use ops::*;
pub use schemas::{
    all_announcements_controller_schemas, all_announcements_registered_controllers,
    announcements_schemas,
};
