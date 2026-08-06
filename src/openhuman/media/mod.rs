//! Media generation and image tool contracts.
//!
//! - [`generation`] — the `media_generate_*` agent tools (image/video via GMI,
//!   proxied through the TinyHumans backend)
//! - [`image`]      — image tool contracts scaffold (currently unwired, #2997)
//!
//! Gated by the `media` feature at the family root (`pub mod media;` in
//! `src/openhuman/mod.rs`), because both children are wholly gated. It is a
//! **surface-only** gate: media generation is backend-proxied over the shared
//! `reqwest`, and the `image` crate is shared with channel upload, so no
//! exclusive dependency is shed. No controller/store/subscriber is tagged
//! `DomainGroup::Media` — this family is agent-tools-only.

pub mod generation;
pub mod image;
