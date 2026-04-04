//! Core-owned app state exposed to the React shell via polling.

mod ops;
mod schemas;

pub use ops::*;
pub use schemas::{
    all_app_state_controller_schemas, all_app_state_registered_controllers, app_state_schemas,
};
