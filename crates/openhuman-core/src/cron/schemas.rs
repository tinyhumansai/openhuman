mod declarations;
mod handlers;

pub use declarations::{all_controller_schemas, all_registered_controllers, schemas};

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
