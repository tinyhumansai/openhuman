//! People: contact resolution + scoring.
//!
//! A5 module. Deterministic resolver maps (imessage handle | email | display
//! name) to a stable `PersonId`. Scoring blends recency × frequency ×
//! reciprocity × depth from interaction rows into a ranked `people.list`.
//!
//! Intentionally self-contained: no dependency on `life_capture`,
//! `chronicle`, `nudges`, or UI. Integration happens in later slices.

pub mod address_book;
pub mod migrations;
pub mod resolver;
pub mod rpc;
pub mod schemas;
pub mod scorer;
pub mod store;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_people_controller_schemas,
    all_registered_controllers as all_people_registered_controllers,
};

#[cfg(test)]
mod tests;
