//! TDLib V8 Runtime Module
//!
//! Provides a V8 JavaScript runtime (via deno_core) for running
//! skill JavaScript code and TDLib via tdweb. Provides a browser-like
//! environment that supports WASM natively.

pub mod ops;
pub mod service;
pub mod storage;

#[allow(unused_imports)]
pub use service::{TdClientAdapter, TdClientConfig, TdUpdate, TdlibV8Service};
pub use storage::IdbStorage;
