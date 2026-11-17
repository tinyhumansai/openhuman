//! Configured and synthesized model catalogs for inference providers.

pub mod catalog_listing;
pub(crate) mod local_runtime;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "catalog_listing_tests.rs"]
mod catalog_listing_tests;

#[cfg(test)]
#[path = "catalog_listing_error_tests.rs"]
mod catalog_listing_error_tests;
