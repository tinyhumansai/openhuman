//! Compatibility view of the shared agent progress projection.
//!
//! Web progress retains its surface-specific lifecycle and sub-agent handling,
//! while the event constructors live in the agent domain so native channels
//! use the same payload caps and structured tool failures.

pub(crate) use crate::agent::turn_progress::*;
