//! Score signals + weighted combine (Phase 2 / #708).
//!
//! Each submodule computes one scoring signal in `[0.0, 1.0]`. [`combine`]
//! aggregates them into a total score using per-signal weights. The output
//! is still `[0.0, 1.0]` after normalisation by total weight.
//!
//! Storing per-signal values alongside the total (via [`ScoreSignals`]) is
//! what makes admission decisions debuggable — when a chunk is dropped, we
//! persist *which* signals fired at what values.

pub mod interaction;
pub mod metadata_weight;
mod ops;
pub mod source_weight;
pub mod token_count;
mod types;
pub mod unique_words;

pub use ops::{combine, compute, entity_density_score};
pub use types::{ScoreSignals, SignalWeights};
