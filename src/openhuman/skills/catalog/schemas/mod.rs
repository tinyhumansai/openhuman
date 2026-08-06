//! RPC schema layer for the skill registry domain.

pub mod controller_schemas;
pub mod handlers;
pub mod wire_types;

pub use controller_schemas::{
    all_skill_registry_controller_schemas, all_skill_registry_registered_controllers,
};
