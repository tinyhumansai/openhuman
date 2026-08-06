//! Phase 2 producer modules for the ambient personalization cache.
//!
//! Each submodule is a distinct producer that writes [`LearningCandidate`]s
//! into [`crate::openhuman::agent::learning::candidate::global()`]. Phase 3
//! (stability detector) drains and aggregates those candidates.
//!
//! | Module | Trigger | Signal class |
//! |--------|---------|-------------|
//! | [`signature`] | `DomainEvent::DocumentCanonicalized` (email) | Identity |
//! | [`heuristics`] | Post-turn hook | Style + Veto |
//! | [`summary_facets`] | LLM summariser output parsing | All classes |

pub mod heuristics;
pub mod signature;
pub mod summary_facets;
