//! Bridge modules for the skill runtime.
//!
//! This module provides bridges between the JavaScript execution environment
//! and the native Rust backend. These bridges allow skills to perform
//! operations that are not natively supported by the JS runtime or require
//! elevated permissions.

pub mod net;
