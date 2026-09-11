//! Config loading, snapshotting, and core runtime-flag helpers.

#[cfg(test)]
#[path = "loader_model_registry_seed_tests_tests.rs"]
mod model_registry_seed_tests;

#[cfg(test)]
#[path = "loader_loader_io_chain_tests_tests.rs"]
mod loader_io_chain_tests;
include!("loader_part_01.rs");
include!("loader_part_02.rs");
