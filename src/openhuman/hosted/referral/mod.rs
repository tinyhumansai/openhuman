//! Referral program RPC adapters (hosted API).

mod ops;
mod schemas;

pub use ops::*;
pub use schemas::{
    all_referral_controller_schemas, all_referral_registered_controllers, referral_schemas,
};
