//! JSON-RPC / CLI controller surface for the bundled local AI stack.
//!
//! This module provides high-level functions for interacting with local AI
//! services such as agent chat, model downloads, summarization, and
//! transcription. These functions are typically invoked via RPC or CLI.

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
include!("ops_part_01.rs");
include!("ops_part_02.rs");
