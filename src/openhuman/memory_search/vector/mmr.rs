//! Maximal Marginal Relevance selection — thin host re-export of
//! `tinycortex::memory::retrieval::mmr` (W5).
//!
//! The MMR algorithm (relevance–diversity tradeoff over embeddings) is the
//! crate's, a byte-identical port. Host consumers (`memory_search::tools`) keep
//! their `memory_search::vector::mmr::{MmrCandidate, MmrResult, mmr_select}`
//! import paths unchanged.

pub use tinycortex::memory::retrieval::mmr::{mmr_select, MmrCandidate, MmrResult};
