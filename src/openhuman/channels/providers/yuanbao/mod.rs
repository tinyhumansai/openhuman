//! Yuanbao (元宝) channel provider.
//!
//! This module is intentionally export-focused. Operational code lives in
//! sibling modules:
//! - [`channel`] wires the provider into the generic OpenHuman `Channel` trait.
//! - [`connection`] owns the WebSocket transport and request correlator.
//! - [`inbound`] owns inbound filtering/extraction.
//! - [`outbound`] owns Yuanbao send/query calls.
//! - [`proto`] / [`proto_biz`] / [`wire`] own hand-written protobuf codecs.

pub mod channel;
pub mod config;
pub mod connection;
pub mod cos;
pub mod errors;
pub mod ids;
pub mod inbound;
pub mod media;
pub mod outbound;
pub mod proto;
pub mod proto_biz;
pub mod proto_constants;
pub mod sign;
pub mod splitter;
pub mod types;
pub mod wire;

pub use channel::YuanbaoChannel;
pub use config::YuanbaoConfig;
