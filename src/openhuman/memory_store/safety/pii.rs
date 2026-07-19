//! Personal-PII detection — thin host re-export of the crate scrubber (W3).
//!
//! The full multilingual national-ID PII module (checksum-gated patterns +
//! Unicode normalization) now lives in `tinycortex::memory::store::safety::pii`;
//! content scrubbing runs inside the crate `sanitize_text`. Host consumers keep
//! their `safety::pii::has_likely_pii` import path.

pub use tinycortex::memory::store::safety::pii::has_likely_pii;
