//! Orchestration domain — ingests tiny.place harness session DMs (stage 3 of the
//! subconscious-orchestration plan) into a durable per-session chat model.
//!
//! - [`types`]: the harness `SessionEnvelopeV1` mirror + persisted session/message model.
//! - [`store`]: SQLite persistence at `<workspace>/orchestration/orchestration.db`.
//! - [`ingest`]: decrypt-once → classify → persist → acknowledge.
//! - [`bus`]: subscriber wiring off `TinyPlaceStreamMessage`.
//!
//! The JSON-RPC read surface (`orchestration.*`) and graph nodes land in later
//! stages; this module is transport/ingest only.

pub mod bus;
pub mod ingest;
pub mod store;
pub mod types;

pub use bus::register_orchestration_ingest_subscriber;
