pub mod auth;
pub mod client;
pub mod policy;
pub mod store;
pub mod sync;
pub mod types;

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
