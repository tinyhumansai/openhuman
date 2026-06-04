//! Agent integration tools.
//!
//! Most integrations proxy through backend endpoints authenticated with the
//! user's session token, so billing, rate limiting, and provider markup stay
//! server-side. Some integrations, such as SearXNG, call user-configured
//! endpoints directly when enabled; those callers must keep configured base URLs
//! trusted because requests leave the local core process.

pub mod client;
pub mod tools;
pub mod types;

pub use client::{build_client, pricing_for_config, IntegrationClient};
pub use types::{
    BackendResponse, IntegrationPricing, IntegrationPricingEntry, PricingIntegrations, ToolScope,
};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
