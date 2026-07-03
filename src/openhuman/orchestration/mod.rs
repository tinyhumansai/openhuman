//! Orchestration domain — ingests tiny.place harness session DMs (stage 3 of the
//! subconscious-orchestration plan) into a durable per-session chat model.
//!
//! - [`types`]: the harness `SessionEnvelopeV1` mirror + persisted session/message model.
//! - [`store`]: SQLite persistence at `<workspace>/orchestration/orchestration.db`.
//! - [`ingest`]: decrypt-once → classify → persist → acknowledge.
//! - [`bus`]: subscriber wiring off `TinyPlaceStreamMessage`.
//!
//! Stage 4 adds the **wake graph** (`graph`), its invocation (`ops`), the
//! front-end agent package (`frontend_agent`), and the front-end decision tools
//! (`tools`). The JSON-RPC read surface (`orchestration.*`) lands in stage 7.

pub mod bus;
pub mod frontend_agent;
pub mod graph;
pub mod ingest;
pub mod ops;
pub mod reasoning_agent;
pub mod schemas;
pub mod steering;
pub mod store;
pub mod tools;
pub mod types;

pub use bus::{
    notify_orchestration_message, register_orchestration_ingest_subscriber,
    register_orchestration_wake_subscriber, subscribe_orchestration_socket,
};
pub use graph::{
    build_orchestration_graph, orchestration_graph_topology, run_orchestration_graph,
    OrchestrationState,
};
pub use schemas::{all_controller_schemas, all_registered_controllers};
