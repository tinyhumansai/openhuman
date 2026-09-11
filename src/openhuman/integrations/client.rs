//! Shared HTTP client for all integration tools.

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
include!("client_part_01.rs");
include!("client_part_02.rs");
