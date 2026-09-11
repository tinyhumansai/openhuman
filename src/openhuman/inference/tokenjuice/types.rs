//! The wire types shared with the separately compiled TinyJuice module.
//!
//! These were declared here — 259 lines of them, under a doc comment saying
//! they were "shared with" the module. They were shared by convention: the
//! module's copy was private to its adapter and the library's copy was the
//! library's, so neither was reachable from here and nothing checked that the
//! three agreed. A field added on one side was a decode failure on the other.
//!
//! `tinyjuice-bus` is that contract as an ordinary crate, and this module is a
//! re-export of it. The names below are the ones ~40 call sites in this crate
//! already use, so the paths are unchanged.
//!
//! `RangeUnit`, `RetrieveRange` and `CacheStats` come from the contract's
//! `wire` module rather than its `types` module — the split there is between
//! values the `tinyjuice` library itself uses and envelopes that exist only on
//! the bus. Nothing here needs to care which is which.

pub use tinyjuice_bus::types::{
    AgentTokenjuiceCompression, CompressOptions, CompressedOutput, CompressorKind, ContentHint,
    ContentKind,
};
pub use tinyjuice_bus::wire::{
    CacheStats, CompactResponse, InstallRequest, RangeUnit, RetrieveRange,
};

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
