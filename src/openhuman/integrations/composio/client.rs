//! Thin HTTP wrapper over the openhuman backend's
//! `/agent-integrations/composio/*` routes.
//!
//! All calls go through the shared
//! [`crate::openhuman::integrations::IntegrationClient`] so they inherit
//! the same Bearer JWT auth, timeout, envelope parsing, and proxy behavior
//! as the other backend-proxied integrations.
//!
//! Logging uses the `[composio]` grep-prefix so all sidecar output for
//! this domain can be filtered in one shot.

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
include!("client_part_01.rs");
include!("client_part_02.rs");
