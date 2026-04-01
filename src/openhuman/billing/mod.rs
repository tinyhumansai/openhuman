//! Billing and payment RPC adapters that thin-wrap the hosted API.
//!
//! Exposes plan lookup, purchase flows, and credit top-ups through the
//! standard controller registry (`openhuman.billing_*`).

mod ops;
mod schemas;

pub use ops::*;
pub use schemas::{
    all_billing_controller_schemas, all_billing_registered_controllers, billing_schemas,
};
