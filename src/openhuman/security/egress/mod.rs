//! Egress spine (privacy epic S2, #4436).
//!
//! Centralizes "what leaves, to where, why" for every external data transfer.
//! See [`types`] for the [`EgressDescriptor`] contract and [`emit`] for the
//! single publish chokepoint. Consumed by the web bridge
//! ([`crate::openhuman::channels::providers::web`]) which surfaces the
//! descriptor to the frontend, and by later privacy slices (disclosure S3,
//! approval S4, identification-risk detector S5, enforcement S7).

pub mod emit;
pub mod enforce;
pub mod types;

pub use emit::{dedup_turn_scope, emit_external_transfer};
pub use enforce::{enforce_egress, local_only_blocks, local_only_tool_block};
pub use types::{DataKind, EgressDescriptor, EgressReason, IdentificationRisk};
