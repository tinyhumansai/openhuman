//! Legacy skill metadata helpers retained after QuickJS runtime removal.

pub mod bus;
pub mod ops;
pub mod schemas;
pub mod types;

pub use ops::*;
pub use schemas::{
    all_skills_controller_schemas, all_skills_registered_controllers, skills_schemas,
};
