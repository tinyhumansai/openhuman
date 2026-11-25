//! Core-owned app state exposed to the React shell via polling.

mod ops;
mod recovery_signal;
mod schemas;

pub use ops::*;
pub use recovery_signal::{config_recovered_this_session, latch_from_config};
pub(crate) use schemas::all_app_state_controller_entries;
pub use schemas::{
    all_app_state_controller_schemas, all_app_state_registered_controllers, app_state_schemas,
};

#[cfg(test)]
pub(crate) use schemas::app_state_parameter_contracts;
