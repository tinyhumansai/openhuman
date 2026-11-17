//! `config` namespace controllers, grouped by the settings area they serve.

#[cfg(test)]
#[path = "controllers_tests.rs"]
mod tests;

mod agent;
mod inference;
mod integrations;
mod registry;
mod workspace;

pub use registry::{all_controller_schemas, all_registered_controllers};

#[cfg(test)]
use agent::handle_update_computer_settings;
#[cfg(test)]
pub(super) use agent::{handle_get_autonomy_settings, handle_update_autonomy_settings};
#[cfg(test)]
pub(super) use workspace::{handle_get_agent_paths, handle_get_data_paths};

#[cfg(test)]
use serde_json::{Map, Value};
