//! JWT helpers for TinyHumans session tokens, from the SDK.
//!
//! The core carries its own copy of these three functions in
//! `openhuman_embed::__host::credentials::jwt` because it needs them with no backend
//! dependency; this module exposes the SDK's originals for hosts that already
//! depend on this crate.

pub use tinyhumans_sdk::jwt::{
    bearer_authorization_value, decode_jwt_exp_unix, decode_jwt_payload,
};
