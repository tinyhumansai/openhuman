pub mod auth;
pub mod client;
pub mod ops;
pub mod policy;
mod schemas;
pub mod store;
pub mod sync;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_canvas_tracker_controller_schemas,
    all_registered_controllers as all_canvas_tracker_registered_controllers,
};

#[cfg(test)]
#[path = "client_tests.rs"]
mod client_tests;

#[cfg(test)]
#[path = "policy_tests.rs"]
mod policy_tests;

#[cfg(test)]
#[path = "store_tests.rs"]
mod store_tests;

#[cfg(test)]
#[path = "sync_tests.rs"]
mod sync_tests;
