//! Registry for external capability providers.
//!
//! This module keeps provider identity and trust metadata generic. It does not
//! know how any provider packages, loads, or executes capabilities; it only
//! normalizes the provider records OpenHuman can use for admission, policy, and
//! diagnostics.

mod registry;
mod types;

pub use registry::{normalize_provider_id, ExternalCapabilityProviderRegistry};
pub use types::{
    ExternalCapabilityProvider, ExternalCapabilityProviderConfig, ExternalCapabilityProvidersConfig,
};
