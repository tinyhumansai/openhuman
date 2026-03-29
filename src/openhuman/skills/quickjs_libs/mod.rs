//! QuickJS Runtime Support Module
//!
//! Provides a QuickJS JavaScript runtime (via rquickjs) for running
//! skill JavaScript code and supporting browser-like shims.
//! environment for skill execution.

pub mod qjs_ops;
pub mod storage;

pub use storage::IdbStorage;
