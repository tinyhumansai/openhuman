//! Long-running Python backend host.
//!
//! `runtime_python` owns interpreter resolution. This module owns the
//! process-level server that keeps Python-backed model modules warm and serves
//! Rust callers over a private JSONL stdio protocol.

pub mod kompress;
pub mod registry;
pub mod server;

pub use kompress::{ensure_kompress, kompress_provisioned, request_kompress, KompressResponse};
pub use registry::{enabled_backends, RuntimePythonBackend};
pub use server::{ensure_started, status, RuntimePythonServer};
pub use tinyruntime_pyserver::{BackendStatus, ServerStatus as RuntimePythonServerStatus};
