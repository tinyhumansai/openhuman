//! Re-exports from the top-level `openhuman::embeddings` module.
//!
//! The canonical embedding logic now lives in `src/openhuman/embeddings/`.
//! This file keeps the old `memory::embeddings::*` import paths working so
//! that existing call sites do not need to change immediately.

pub use crate::openhuman::embeddings::*;
