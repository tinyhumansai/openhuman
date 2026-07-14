//! Concrete subconscious worlds. Each profile file implements
//! [`SubconsciousProfile`](super::profile::SubconsciousProfile) for one world;
//! the generic [`SubconsciousInstance`](super::instance::SubconsciousInstance)
//! runner ticks it. Adding a world is a new file here, not a new engine.

pub mod memory;
