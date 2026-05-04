//! Legacy skill metadata helpers retained after QuickJS runtime removal.

pub mod bus;
pub mod inject;
pub mod ops;
pub mod ops_create;
pub mod ops_discover;
pub mod ops_install;
pub mod ops_parse;
pub mod ops_types;
pub mod schemas;
pub mod types;

pub use ops::*;
pub use schemas::{
    all_skills_controller_schemas, all_skills_registered_controllers, skills_schemas,
};
